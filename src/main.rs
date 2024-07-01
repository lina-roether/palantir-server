use std::time::{SystemTime, UNIX_EPOCH};

use api_access::{ApiAccessManager, ApiPermissions};
use messages::{Message, MessageBody, SessionStateMsgBody, SessionUser, SessionUserRole};
use uuid::Uuid;

mod api_access;
mod media;
mod messages;
mod playback;
mod session;
mod user;

fn main() {
    pretty_env_logger::init();

    let access = ApiAccessManager::new(None);
    dbg!(access.acquire_permissions(Some("test"), ApiPermissions::join()));
}
