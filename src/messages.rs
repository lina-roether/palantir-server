use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::utils::timestamp;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionLoginMsgBodyV1 {
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionPongMsgBodyV1 {
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ConnectionClosedReasonV1 {
    #[serde(rename = "auth_failed")]
    AuthFailed,

    #[serde(rename = "server_error")]
    ServerError,

    #[serde(rename = "session_closed")]
    SessionClosed,

    #[serde(rename = "timeout")]
    Timeout,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionClosedMsgBodyV1 {
    pub reason: ConnectionClosedReasonV1,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionClientErrorMsgBodyV1 {
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStartMsgBodyV1 {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionJoinMsgBodyV1 {
    pub id: Uuid,
    pub name: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum SessionUserRoleV1 {
    #[serde(rename = "host")]
    Host,

    #[serde(rename = "guest")]
    Guest,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionUserV1 {
    pub name: String,
    pub role: SessionUserRoleV1,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStateMsgBodyV1 {
    pub id: Uuid,
    pub password: String,
    pub users: Vec<SessionUserV1>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum SessionTerminateReasonV1 {
    #[serde(rename = "closed_by_host")]
    ClosedByHost,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionTerminatedMsgBodyV1 {
    pub reason: SessionTerminateReasonV1,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MediaSelectMsgBodyV1 {
    pub page_href: String,
    pub frame_href: String,
    pub element_query: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlaybackSyncMsgBodyV1 {
    pub active_sync: bool,
    pub playing: bool,
    pub time: u64,
    pub timestamp: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "m")]
pub enum MessageBody {
    #[serde(rename = "connection::login/v1")]
    ConnectionLoginV1(ConnectionLoginMsgBodyV1),

    #[serde(rename = "connection::login_ack/v1")]
    ConnectionLoginAckV1,

    #[serde(rename = "connection::ping/v1")]
    ConnectionPingV1,

    #[serde(rename = "connection::pong/v1")]
    ConnectionPongV1(ConnectionPongMsgBodyV1),

    #[serde(rename = "connection::client_error/v1")]
    ConnectionClientErrorV1(ConnectionClientErrorMsgBodyV1),

    #[serde(rename = "connection::closed/v1")]
    ConnectionClosedV1(ConnectionClosedMsgBodyV1),

    #[serde(rename = "session::start/v1")]
    SessionStartV1(SessionStartMsgBodyV1),

    #[serde(rename = "session::stop/v1")]
    SessionStopV1,

    #[serde(rename = "session::join/v1")]
    SessionJoinV1(SessionJoinMsgBodyV1),

    #[serde(rename = "session::leave/v1")]
    SessionLeaveV1,

    #[serde(rename = "session::terminated/v1")]
    SessionTerminatedV1(SessionTerminatedMsgBodyV1),

    #[serde(rename = "session::state/v1")]
    SessionStateV1(SessionStateMsgBodyV1),

    #[serde(rename = "session::keepalive/v1")]
    SessionKeepaliveV1,

    #[serde(rename = "media::select/v1")]
    MediaSelectV1(MediaSelectMsgBodyV1),

    #[serde(rename = "playback::sync/v1")]
    PlaybackSyncV1(PlaybackSyncMsgBodyV1),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "t")]
    pub timestamp: u64,

    #[serde(flatten)]
    pub body: MessageBody,
}

impl Message {
    pub fn new(body: MessageBody) -> Self {
        Self {
            timestamp: timestamp(),
            body,
        }
    }
}
