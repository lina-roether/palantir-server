use std::sync::Arc;

use api_access::ApiAccessManager;
use config::read_config;
use connection::Connection;
use listener::Listener;
use log::error;

mod api_access;
mod config;
mod connection;
mod listener;
mod messages;
mod session;
mod user;
mod utils;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let config = read_config(None);

    let access = Arc::new(ApiAccessManager::new(Arc::clone(&config)));

    let server = match Listener::bind(config).await {
        Ok(server) => server,
        Err(err) => {
            error!("Failed to start server: {err:?}");
            return;
        }
    };

    server
        .listen(move |channel| {
            let access = Arc::clone(&access);
            async move {
                let mut conn = Connection::new(channel).await;
                conn.init(&access).await;
                Ok(())
            }
        })
        .await;
}
