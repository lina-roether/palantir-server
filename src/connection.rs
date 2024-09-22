use std::{fmt::Display, sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use futures::executor;
use futures_util::Future;
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
                if let Err(err) =
                    Self::handle_connection(addr.to_string(), stream, handler_ref).await
                {
                    error!("Error during connection with {addr}: {err:?}");
                }
            });
        }
    }

    async fn handle_connection<F: Future<Output = anyhow::Result<()>>>(
        name: String,
        stream: TcpStream,
        handler: Arc<impl Fn(Connection) -> F>,
    ) -> anyhow::Result<()> {
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .context("Failed to accept websocket connection")?;

        handler(Connection::new(name, ws)).await?;

        Ok(())
    }
}

pub struct Connection {
    open: bool,
    name: String,
    username: Option<String>,
    permissions: ApiPermissions,
    channel: MessageChannel<WebSocketStream<TcpStream>>,
}

#[derive(Debug, Clone)]
pub struct PingResult {
    pub time_offset: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    ServerError,
    Unauthorized,
    RoomClosed,
}

impl Connection {
    const LOGIN_TIMEOUT: Duration = Duration::from_secs(3);
    const PING_TIMEOUT: Duration = Duration::from_secs(1);

    pub fn new(name: String, ws: WebSocketStream<TcpStream>) -> Self {
        debug!("Creating connection {name}");
        Self {
            open: true,
            name,
            username: None,
            permissions: ApiPermissions::default(),
            channel: MessageChannel::new(ws),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn username(&self) -> &str {
        self.username
            .as_ref()
            .map(String::as_ref)
            .unwrap_or("Not logged in")
    }

    pub fn permissions(&self) -> &ApiPermissions {
        &self.permissions
    }

    pub async fn init(&mut self, access_mgr: &ApiAccessManager) -> anyhow::Result<()> {
        debug!("Waiting for login message on connection {}...", self.name);
        'wait_for_login: loop {
            match timeout(Self::LOGIN_TIMEOUT, self.recv()).await {
                Ok(None) => return Err(anyhow!("Connection closed before logging in")),
                Ok(Some(Message {
                    body: MessageBody::ConnectionLoginV1(body),
                    ..
                })) => {
                    self.username = Some(body.username);
                    self.permissions = access_mgr.get_permissions(body.api_key.as_deref());
                    if !self.permissions.connect {
                        self.close(CloseReason::Unauthorized, "Unauthorized")
                            .await
                            .context("Failed to close unauthorized connection")?;
                        return Err(anyhow!("Unauthorized"));
                    } else {
                        self.send(Message::new(MessageBody::ConnectionLoginAckV1))
                            .await
                            .context("Failed to send login ack message")?;
                        break 'wait_for_login;
                    }
                }
                Ok(Some(Message { .. })) => self.send_error("Expected login message").await,
                Err(timeout_err) => {
                    let err = anyhow!(timeout_err).context("Login message not received in time!");
                    self.close(CloseReason::Unauthorized, &err)
                        .await
                        .context("Failed to close connection after failed authentication")?;
                    return Err(err);
                }
            }
        }
        debug!("Connection {} logged in successfully", self.name);
        Ok(())
    }

    pub async fn send(&mut self, message: Message) -> anyhow::Result<()> {
        self.channel.send(message).await?;
        Ok(())
    }

    pub async fn send_error(&mut self, message: impl Display) {
        let _ = self
            .send(Message::new(MessageBody::ConnectionClientErrorV1(
                ConnectionClientErrorMsgBodyV1 {
                    message: message.to_string(),
                },
            )))
            .await;
    }

    pub async fn recv(&mut self) -> Option<Message> {
        if !self.open {
            return None;
        }
        loop {
            let Some(msg_res) = self.channel.recv().await else {
                self.close_silent().await;
                return None;
            };
            match msg_res {
                Ok(Message {
                    body: MessageBody::ConnectionPingV1,
                    ..
                }) => {
                    if let Err(err) = self.send(Message::new(MessageBody::ConnectionPongV1)).await {
                        error!("Failed to send pong to client {}: {err:?}", self.name);
                    }
                }
                Ok(Message {
                    body: MessageBody::ConnectionKeepaliveV1,
                    ..
                }) => {
                    // do nothing
                }
                Ok(msg) => return Some(msg),
                Err(err) => {
                    debug!(
                        "Received malformed message from client {}: {err:?}",
                        self.name
                    );
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
                    let expected_timestamp = start_time + u64::abs_diff(start_time, end_time) / 2;
                    let time_offset =
                        u64::wrapping_sub(actual_timestamp, expected_timestamp) as i64;
                    debug!(
                        "Pinged client {}, and found a time offset of {time_offset}ms",
                        self.name
                    );
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
                        CloseReason::RoomClosed => ConnectionClosedReasonV1::RoomClosed,
                        CloseReason::Unauthorized => ConnectionClosedReasonV1::Unauthorized,
                    },
                    message: message.to_string(),
                },
            )))
            .await;
        self.close_silent().await;
        result
    }

    async fn close_silent(&mut self) {
        self.open = false;
        if let Err(err) = self.channel.close().await {
            error!("Failed to close websocket {}: {err:?}", self.name);
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
            error!("Failed to close connection {} in drop: {err:?}", self.name)
        }
    }
}
