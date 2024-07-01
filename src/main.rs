use std::sync::Arc;

use config::read_config;
use log::error;
use server::start_server;

mod api_access;
mod config;
mod media;
mod messages;
mod playback;
mod server;
mod session;
mod user;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let config = read_config(None);

    if let Err(err) = start_server(Arc::clone(&config)).await {
        error!("Failed to start server: {err:?}");
    }
}
