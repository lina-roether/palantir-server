use std::sync::Arc;

use api_access::{ApiAccessConfig, ApiAccessManager, ApiAccessPolicy};
use config::{read_config, Config};
use connection::{CloseReason, ConnectionListener};

mod api_access;
mod config;
mod connection;
mod messages;
mod session;
mod utils;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    // let config = read_config(None);
    let config = Arc::new(Config {
        api_access: ApiAccessConfig {
            policy: ApiAccessPolicy {
                restrict_connect: false,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    });

    let access_mgr = Arc::new(ApiAccessManager::new(Arc::clone(&config)));

    let listener = ConnectionListener::bind(Arc::clone(&config)).await.unwrap();
    listener
        .listen(move |mut conn| {
            let access_mgr = Arc::clone(&access_mgr);
            async move {
                conn.init(&access_mgr).await.unwrap();
                conn.close(CloseReason::SessionClosed, "Session closed")
                    .await
                    .unwrap();
                Ok(())
            }
        })
        .await;
}
