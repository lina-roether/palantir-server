use std::sync::Arc;

use api_access::{ApiAccessConfig, ApiAccessManager, ApiAccessPolicy};
use config::Config;
use connection::ConnectionListener;
use room::RoomManager;
use session::Session;
use tokio::sync;

mod api_access;
mod config;
mod connection;
mod messages;
mod room;
mod session;
mod utils;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    // let config = read_config(None);
    let config = Config {
        api_access: ApiAccessConfig {
            api_policy: ApiAccessPolicy {
                restrict_connect: false,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };

    let access_mgr = Arc::new(ApiAccessManager::new(config.api_access));
    let room_mgr = Arc::new(sync::Mutex::new(RoomManager::new()));

    let listener = ConnectionListener::bind(config.server).await.unwrap();
    listener
        .listen(move |mut conn| {
            let access_mgr = Arc::clone(&access_mgr);
            let room_mgr = Arc::clone(&room_mgr);
            async move {
                conn.init(&access_mgr).await.unwrap();

                let mut session = Session::new(conn, room_mgr);
                session.run().await;

                Ok(())
            }
        })
        .await;
}
