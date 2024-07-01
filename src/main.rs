use std::sync::Arc;

use api_access::{ApiAccessManager, ApiPermissions};
use config::read_config;

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

    let mut config = read_config(None);

    let access = ApiAccessManager::new(Arc::clone(&config));
    dbg!(access.acquire_permissions(Some("test"), ApiPermissions::join()));
}
