use std::{fmt::Display, sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use futures::executor;
use futures_util::{Future, SinkExt, StreamExt};
use log::{debug, error, info};
use serde::Deserialize;
use tokio::{
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_tungstenite::WebSocketStream;

use crate::{
    api_access::{ApiAccessManager, ApiPermissions},
    config::Config,
    messages::{
        ConnectionClientErrorMsgBodyV1, ConnectionClosedMsgBodyV1, ConnectionClosedReasonV1,
        Message, MessageBody, MessageChannel,
    },
    utils::timestamp,
};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    listen_on: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_on: "127.0.0.1:8069".to_string(),
        }
    }
}

pub struct ConnectionListener {
    config: ServerConfig,
    listener: TcpListener,
}

impl ConnectionListener {
    pub async fn bind(config: Arc<Config>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(&config.server.listen_on)
            .await
            .context("Failed to start TCP server")?;
        Ok(Self {
            listener,
            config: config.server.clone(),
        })
    }

    pub async fn listen<F: Future<Output = anyhow::Result<()>> + Send>(
        &self,
        handler: impl Fn(Connection) -> F + Send + Sync + 'static,
    ) {
        info!("Server listening on {}...", self.config.listen_on);

        let handler = Arc::new(handler);

        loop {
            let (stream, addr) = match self.listener.accept().await {
                Ok(val) => val,
                Err(err) => {
                    error!("TCP connection failed: {err:?}");
                    continue;
                }
            };
            let handler_ref = Arc::clone(&handler);
            tokio::spawn(async move {
                if let Err(err) = Self::handle_connection(stream, handler_ref).await {
                    error!("Error during connection with {addr}: {err:?}");
                }
            });
        }
    }

    async fn handle_connection<F: Future<Output = anyhow::Result<()>>>(
        stream: TcpStream,
        handler: Arc<impl Fn(Connection) -> F>,
    ) -> anyhow::Result<()> {
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .context("Failed to accept websocket connection")?;

        handler(Connection::new(ws)).await?;

        Ok(())
    }
}

pub struct Connection {
    open: bool,
    permissions: ApiPermissions,
    channel: MessageChannel<WebSocketStream<TcpStream>>,
}

#[derive(Debug, Clone)]
pub struct PingResult {
    time_offset: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    ServerError,
    AuthFailed,
    SessionClosed,
}

impl Connection {
    const LOGIN_TIMEOUT: Duration = Duration::from_secs(3);
    const PING_TIMEOUT: Duration = Duration::from_secs(1);

    pub fn new(ws: WebSocketStream<TcpStream>) -> Self {
        Self {
            open: true,
            permissions: ApiPermissions::default(),
            channel: MessageChannel::new(ws),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn permissions(&self) -> &ApiPermissions {
        &self.permissions
    }

    pub async fn init(&mut self, access_mgr: &ApiAccessManager) -> anyhow::Result<()> {
        self.send(Message::new(MessageBody::ConnectionHelloV1))
            .await?;

        loop {
            match timeout(Self::LOGIN_TIMEOUT, self.recv()).await {
                Ok(None) => return Err(anyhow!("Connection closed before logging in")),
                Ok(Some(Message {
                    body: MessageBody::ConnectionLoginV1(body),
                    ..
                })) => {
                    self.permissions = access_mgr.get_permissions(body.api_key.as_deref());
                    if !self.permissions.connect {
                        self.close(CloseReason::AuthFailed, "Unauthorized")
                            .await
                            .context("Failed to close unauthorized connection")?;
                        return Err(anyhow!("Unauthorized"));
                    } else {
                        self.send(Message::new(MessageBody::ConnectionLoginAckV1))
                            .await
                            .context("Failed to send login ack message")?;
                    }
                }
                Ok(Some(Message { .. })) => self.send_error("Expected login message").await,
                Err(timeout_err) => {
                    let err = anyhow!(timeout_err).context("Login message not received in time!");
                    self.close(CloseReason::AuthFailed, &err)
                        .await
                        .context("Failed to close connection after failed authentication")?;
                    return Err(err);
                }
            }
        }
    }

    pub async fn send(&mut self, message: Message) -> anyhow::Result<()> {
        self.channel.send(message).await?;
        Ok(())
    }

    pub async fn send_error(&mut self, message: impl Display) {
        let result = self
            .send(Message::new(MessageBody::ConnectionClientErrorV1(
                ConnectionClientErrorMsgBodyV1 {
                    message: message.to_string(),
                },
            )))
            .await;
        if let Err(err) = result {
            error!("Failed to send client error message: {err:?}")
        }
    }

    pub async fn recv(&mut self) -> Option<Message> {
        if !self.open {
            return None;
        }
        loop {
            let Some(msg_res) = self.channel.next().await else {
                self.close_immediately().await;
                return None;
            };
            match msg_res {
                Ok(Message {
                    body: MessageBody::ConnectionPingV1,
                    ..
                }) => {
                    if let Err(err) = self.send(Message::new(MessageBody::ConnectionPongV1)).await {
                        error!("Failed to send pong: {err:?}");
                    }
                }
                Ok(msg) => return Some(msg),
                Err(err) => {
                    debug!("Received malformed message from client: {err:?}");
                    self.send_error(err.to_string()).await;
                }
            }
        }
    }

    pub async fn ping(&mut self) -> anyhow::Result<Option<PingResult>> {
        let start_time = timestamp();
        self.send(Message {
            timestamp: start_time,
            body: MessageBody::ConnectionPingV1,
        })
        .await?;

        loop {
            match timeout(Self::PING_TIMEOUT, self.recv()).await {
                Ok(None) => return Ok(None),
                Ok(Some(Message {
                    timestamp: actual_timestamp,
                    body: MessageBody::ConnectionPongV1,
                })) => {
                    let end_time = timestamp();
                    let expected_timestamp = u64::abs_diff(start_time, end_time) / 2;
                    let time_offset =
                        u64::wrapping_sub(actual_timestamp, expected_timestamp) as i64;
                    return Ok(Some(PingResult { time_offset }));
                }
                Ok(Some(Message { .. })) => {
                    self.send_error("Expected pong message").await;
                }
                Err(timeout_err) => {
                    let err = anyhow!(timeout_err).context("Pong message not received in time!");
                    self.send_error(&err).await;
                    return Err(err);
                }
            }
        }
    }

    pub async fn close(
        &mut self,
        reason: CloseReason,
        message: impl Display,
    ) -> anyhow::Result<()> {
        if !self.is_open() {
            return Ok(());
        }
        let result = self
            .send(Message::new(MessageBody::ConnectionClosedV1(
                ConnectionClosedMsgBodyV1 {
                    reason: match reason {
                        CloseReason::ServerError => ConnectionClosedReasonV1::ServerError,
                        CloseReason::SessionClosed => ConnectionClosedReasonV1::SessionClosed,
                        CloseReason::AuthFailed => ConnectionClosedReasonV1::AuthFailed,
                    },
                    message: message.to_string(),
                },
            )))
            .await;
        self.close_immediately().await;
        result
    }

    async fn close_immediately(&mut self) {
        self.open = false;
        if let Err(err) = self.channel.close().await {
            error!("Failed to close websocket: {err:?}");
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.is_open() {
            return;
        }
        let close_future = self.close(CloseReason::ServerError, "Connection terminated");
        if let Err(err) = executor::block_on(close_future) {
            error!("Failed to close connection in drop: {err:?}")
        }
    }
}
