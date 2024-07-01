use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::SinkExt;
use log::{debug, error, info};
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite;

use crate::config::Config;

#[derive(Debug, Deserialize)]
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

pub async fn start_server(config: Arc<Config>) -> Result<()> {
    let listener = TcpListener::bind(&config.server.listen_on)
        .await
        .context("Failed to start TCP server")?;

    info!("Server listening on {}...", config.server.listen_on);

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(val) => val,
            Err(err) => {
                error!("TCP connection failed: {err:?}");
                continue;
            }
        };
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream).await {
                error!("Error during connection with {addr}: {err:?}");
            }
        });
    }
}

async fn handle_connection(stream: TcpStream) -> Result<()> {
    let mut ws = tokio_tungstenite::accept_async(stream)
        .await
        .context("Failed to accept websocket connection")?;

    ws.send(tungstenite::Message::Text("Moin".to_string()))
        .await?;

    ws.close(None).await?;

    Ok(())
}
