use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, error, info};
use serde::Deserialize;
use tokio::{io::AsyncWriteExt, net::TcpListener};

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
        let (mut stream, addr) = match listener.accept().await {
            Ok(val) => val,
            Err(err) => {
                error!("TCP connection failed: {err}");
                continue;
            }
        };
        stream.shutdown().await.unwrap();
    }
}
