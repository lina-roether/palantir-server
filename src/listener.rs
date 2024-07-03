use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Error, Result};
use futures_util::{
    stream::{SplitSink, SplitStream},
    Future, SinkExt, StreamExt,
};
use log::{error, info};
use serde::Deserialize;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{tungstenite, WebSocketStream};

use crate::{
    config::Config,
    messages::{
        ConnectionClientErrorMsgBodyV1, ConnectionClosedMsgBodyV1, ConnectionClosedReasonV1,
        Message, MessageBody,
    },
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

pub struct Listener {
    config: ServerConfig,
    listener: TcpListener,
}

impl Listener {
    pub async fn bind(config: Arc<Config>) -> Result<Self> {
        let listener = TcpListener::bind(&config.server.listen_on)
            .await
            .context("Failed to start TCP server")?;
        Ok(Self {
            listener,
            config: config.server.clone(),
        })
    }

    pub async fn listen<F: Future<Output = Result<()>> + Send>(
        &self,
        handler: impl Fn(MessageChannel) -> F + Send + Sync + 'static,
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

    async fn handle_connection<F: Future<Output = Result<()>>>(
        stream: TcpStream,
        handler: Arc<impl Fn(MessageChannel) -> F>,
    ) -> Result<()> {
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .context("Failed to accept websocket connection")?;

        handler(MessageChannel::new(ws)).await?;

        Ok(())
    }
}

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

pub enum ConnectionClosedReason {
    AuthFailed,
    ServerError,
    SessionClosed,
}

impl From<ConnectionClosedReason> for ConnectionClosedReasonV1 {
    fn from(value: ConnectionClosedReason) -> Self {
        match value {
            ConnectionClosedReason::AuthFailed => Self::AuthFailed,
            ConnectionClosedReason::ServerError => Self::ServerError,
            ConnectionClosedReason::SessionClosed => Self::SessionClosed,
        }
    }
}

pub struct MessageChannel {
    ws: SplitStream<WebSocketStream<TcpStream>>,
    out_send: Sender<SendCommand>,
    send_thread: JoinHandle<()>,
}

enum SendCommand {
    Send(Message),
    Close,
}

impl MessageChannel {
    fn new(ws: WebSocketStream<TcpStream>) -> Self {
        let (ws_sink, ws_stream) = ws.split();
        let (out_send, out_recv) = channel(64);
        Self {
            ws: ws_stream,
            out_send,
            send_thread: tokio::spawn(Self::send_thread(ws_sink, out_recv)),
        }
    }

    async fn next_single(&mut self) -> Option<Result<Message>> {
        let raw_msg_res = self.ws.next().await?;

        let raw_msg = match raw_msg_res {
            Ok(raw_msg) => raw_msg,
            Err(err) => return Some(Err(err.into())),
        };

        let tungstenite::Message::Binary(data) = raw_msg else {
            return Some(Err(Error::msg("Received non-binary data")));
        };

        match rmp_serde::from_slice(&data) {
            Ok(message) => Some(Ok(message)),
            Err(err) => Some(Err(err.into())),
        }
    }

    pub async fn next(&mut self) -> Option<Message> {
        loop {
            match self.next_single().await? {
                Ok(msg) => return Some(msg),
                Err(err) => self.send_client_error(err.to_string()).await,
            }
        }
    }

    pub async fn next_response(&mut self) -> Option<Message> {
        let Ok(msg_opt) = timeout(RESPONSE_TIMEOUT, self.next()).await else {
            self.send_client_error("Response timeout exceeded".to_string())
                .await;
            return None;
        };
        msg_opt
    }

    pub async fn send(&mut self, message: Message) -> Result<()> {
        self.out_send.send(SendCommand::Send(message)).await?;
        Ok(())
    }

    pub fn sender(&self) -> MessageSender {
        MessageSender {
            out_send: self.out_send.clone(),
        }
    }

    pub async fn send_client_error(&mut self, message: String) {
        let _ = self
            .send(Message::new(MessageBody::ConnectionClientErrorV1(
                ConnectionClientErrorMsgBodyV1 { message },
            )))
            .await;
    }

    pub async fn close(&mut self, reason: ConnectionClosedReason, message: String) {
        let _ = self
            .send(Message::new(MessageBody::ConnectionClosedV1(
                ConnectionClosedMsgBodyV1 {
                    reason: reason.into(),
                    message,
                },
            )))
            .await;
        let _ = self.out_send.send(SendCommand::Close).await;
    }

    async fn send_thread(
        mut ws: SplitSink<WebSocketStream<TcpStream>, tungstenite::Message>,
        mut out_recv: Receiver<SendCommand>,
    ) {
        while let Some(cmd) = out_recv.recv().await {
            match cmd {
                SendCommand::Send(message) => {
                    if let Err(err) = Self::send_direct(&mut ws, &message).await {
                        error!("Failed to send message: {err:?}");
                        let _ = Self::send_direct(
                            &mut ws,
                            &Message::new(MessageBody::ConnectionClosedV1(
                                ConnectionClosedMsgBodyV1 {
                                    reason: ConnectionClosedReasonV1::ServerError,
                                    message: err.to_string(),
                                },
                            )),
                        )
                        .await;
                        if let Err(err) = ws.close().await {
                            error!("Failed to close socket after send error: {err:?}");
                        }
                    }
                }
                SendCommand::Close => {
                    if let Err(err) = ws.close().await {
                        error!("Failed to close socket: {err:?}");
                    }
                }
            }
        }
    }

    async fn send_direct(
        ws: &mut SplitSink<WebSocketStream<TcpStream>, tungstenite::Message>,
        message: &Message,
    ) -> Result<()> {
        let data = rmp_serde::to_vec(&message).context("Failed to serialize message")?;
        ws.send(tungstenite::Message::Binary(data)).await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MessageSender {
    out_send: Sender<SendCommand>,
}

impl MessageSender {
    pub async fn send(&mut self, message: Message) -> Result<()> {
        self.out_send.send(SendCommand::Send(message)).await?;
        Ok(())
    }
}
