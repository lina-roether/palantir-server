use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthLoginMsgBody {
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimingPongMsgBody {
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStartMsgBody {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionJoinMsgBody {
    pub id: Uuid,
    pub name: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum SessionUserRole {
    #[serde(rename = "host")]
    Host,

    #[serde(rename = "guest")]
    Guest,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionUser {
    pub name: String,
    pub role: SessionUserRole,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStateMsgBody {
    pub id: Uuid,
    pub password: String,
    pub users: Vec<SessionUser>,
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
#[serde(tag = "m")]
pub enum MessageBody {
    #[serde(rename = "auth::login")]
    AuthLogin(AuthLoginMsgBody),

    #[serde(rename = "auth::ack")]
    AuthAck,

    #[serde(rename = "timing::ping")]
    TimingPing,

    #[serde(rename = "timing::pong")]
    TimingPong(TimingPongMsgBody),

    #[serde(rename = "session::start")]
    SessionStart(SessionStartMsgBody),

    #[serde(rename = "session::stop")]
    SessionStop,

    #[serde(rename = "session::join")]
    SessionJoin(SessionJoinMsgBody),

    #[serde(rename = "session::leave")]
    SessionLeave,

    #[serde(rename = "session::state")]
    SessionState(SessionStateMsgBody),

    #[serde(rename = "session::keepalive")]
    SessionKeepalive,

    #[serde(rename = "media::select")]
    MediaSelect(MediaSelectMsgBody),

    #[serde(rename = "playback::sync")]
    PlaybackSync(PlaybackSyncMsgBody),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "v")]
    pub version: u16,

    #[serde(rename = "t")]
    pub timestamp: u64,

    #[serde(flatten)]
    pub body: MessageBody,
}
