use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct UserLoginMsgBody {
    pub api_key: Option<Vec<u8>>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStartMsgBody {
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionJoinMsgBody {
    pub id: Uuid,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MediaSelectMsgBody {
    pub page_href: String,
    pub frame_href: String,
    pub element_query: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlaybackSyncMsgBody {
    pub active_sync: bool,
    pub playing: bool,
    pub time: u64,
    pub timestamp: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminGenApiKeyMsgBody {
    pub join: bool,
    pub host: bool,
    pub admin: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum MessageBody {
    #[serde(rename = "user::login")]
    UserLogin(UserLoginMsgBody),

    #[serde(rename = "session::start")]
    SessionStart(SessionStartMsgBody),

    #[serde(rename = "session::stop")]
    SessionStop,

    #[serde(rename = "session::join")]
    SessionJoin(SessionJoinMsgBody),

    #[serde(rename = "session::leave")]
    SessionLeave,

    #[serde(rename = "media::select")]
    MediaSelect(MediaSelectMsgBody),

    #[serde(rename = "playback::sync")]
    PlaybackSync(PlaybackSyncMsgBody),

    #[serde(rename = "admin::gen_api_key")]
    AdminGenApiKey(AdminGenApiKeyMsgBody),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub version: u16,

    #[serde(flatten)]
    pub body: MessageBody,
}

impl TryFrom<&Message> for tungstenite::Message {
    type Error = rmp_serde::encode::Error;

    fn try_from(value: &Message) -> Result<Self, Self::Error> {
        Ok(tungstenite::Message::Binary(rmp_serde::to_vec(&value)?))
    }
}
