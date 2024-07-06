use std::sync::Arc;

use api_access::ApiAccessManager;
use config::read_config;
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

    let config = read_config(None);

    let access_mgr = Arc::new(ApiAccessManager::new(Arc::clone(&config)));

    let listener = ConnectionListener::bind(Arc::clone(&config)).await.unwrap();
    listener
        .listen(move |mut conn| {
            let access_mgr = Arc::clone(&access_mgr);
            async move {
                let ping_res = conn.init(&access_mgr).await;
                let _ = dbg!(ping_res);
                conn.close(CloseReason::SessionClosed, "Session closed")
                    .await
                    .unwrap();
                Ok(())
            }
        })
        .await;
}
