use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::{Context as _, Error, Result};
use futures_util::{
    stream::{SplitSink, SplitStream},
    Future, Sink, SinkExt, Stream, StreamExt,
};
use log::{error, info};
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite, WebSocketStream};

use crate::{config::Config, messages::Message};

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
        handler: impl Fn(MessageStream, MessageSink) -> F + Send + Sync + 'static,
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
        handler: Arc<impl Fn(MessageStream, MessageSink) -> F>,
    ) -> Result<()> {
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .context("Failed to accept websocket connection")?;

        let (sink, stream) = ws.split();
        handler(MessageStream { stream }, MessageSink { sink }).await?;

        Ok(())
    }
}

pub struct MessageStream {
    stream: SplitStream<WebSocketStream<TcpStream>>,
}

impl Stream for MessageStream {
    type Item = Result<Message>;

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.poll_next_unpin(cx).map(|msg_res_opt| {
            let Some(msg_res) = msg_res_opt else {
                return None;
            };
            let msg = match msg_res {
                Ok(msg) => msg,
                Err(err) => return Some(Err(err.into())),
            };
            let tungstenite::Message::Binary(data) = msg else {
                return Some(Err(Error::msg("Received non-binary data")));
            };
            let parsed_message: Message =
                match rmp_serde::from_slice(&data).context("Message format error") {
                    Ok(msg) => msg,
                    Err(err) => return Some(Err(err)),
                };
            Some(Ok(parsed_message))
        })
    }
}

pub struct MessageSink {
    sink: SplitSink<WebSocketStream<TcpStream>, tungstenite::Message>,
}

impl Sink<Message> for MessageSink {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.sink
            .poll_ready_unpin(cx)
            .map(|r| r.map_err(Into::into))
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<()> {
        self.sink
            .start_send_unpin(tungstenite::Message::binary(rmp_serde::to_vec(&item)?))?;
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.sink
            .poll_flush_unpin(cx)
            .map(|r| r.map_err(Into::into))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.sink
            .poll_close_unpin(cx)
            .map(|r| r.map_err(Into::into))
    }
}
