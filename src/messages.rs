use std::{
    error::Error,
    pin::Pin,
    task::{Context, Poll},
    time::SystemTime,
};

use anyhow::{anyhow, Context as _};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

use crate::utils::timestamp;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionLoginMsgBodyV1 {
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ConnectionClosedReasonV1 {
    #[serde(rename = "unauthorized")]
    Unauthorized,

    #[serde(rename = "server_error")]
    ServerError,

    #[serde(rename = "session_closed")]
    SessionClosed,

    #[serde(rename = "timeout")]
    Timeout,

    #[serde(rename = "unknown")]
    Unknown,
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

    #[serde(rename = "unauthorized")]
    Unauthorized,

    #[serde(rename = "server_error")]
    ServerError,
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
    ConnectionPongV1,

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MessageFormat {
    Json,

    #[default]
    Msgpack,
}

pub struct MessageChannel<S> {
    format: MessageFormat,
    ws: S,
}

impl<S> MessageChannel<S> {
    pub fn new(ws: S) -> Self {
        Self {
            format: MessageFormat::default(),
            ws,
        }
    }
}

impl<S> Sink<Message> for MessageChannel<S>
where
    S: Sink<tungstenite::Message> + Unpin,
    S::Error: Error + Send + Sync + 'static,
{
    type Error = anyhow::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.ws.poll_ready_unpin(cx).map_err(anyhow::Error::from)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.ws.poll_flush_unpin(cx).map_err(anyhow::Error::from)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.ws.poll_close_unpin(cx).map_err(anyhow::Error::from)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let msg = match self.format {
            MessageFormat::Msgpack => tungstenite::Message::Binary(
                rmp_serde::to_vec(&item).context("Failed to serialize message as MsgPack")?,
            ),
            MessageFormat::Json => tungstenite::Message::Text(
                serde_json::to_string(&item).context("Failed to serialize message as JSON")?,
            ),
        };
        self.ws.start_send_unpin(msg)?;
        Ok(())
    }
}

impl<S> Stream for MessageChannel<S>
where
    S: Sink<tungstenite::Message>
        + Stream<Item = tungstenite::Result<tungstenite::Message>>
        + Unpin,
    S::Error: Error + Send + Sync,
{
    type Item = anyhow::Result<Message>;

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.ws.size_hint()
    }

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.ws.poll_next_unpin(cx).map(|msg| {
            let Some(msg) = msg else {
                return None;
            };
            let msg = match msg {
                Ok(msg) => msg,
                Err(err) => return Some(Err(err.into())),
            };
            let deserialized_res: anyhow::Result<Message> = match msg {
                tungstenite::Message::Binary(data) => {
                    self.format = MessageFormat::Msgpack;
                    rmp_serde::from_slice(&data).map_err(|err| {
                        anyhow!(err).context("Failed to deserialize binary message as MsgPack")
                    })
                }
                tungstenite::Message::Text(data) => {
                    self.format = MessageFormat::Json;
                    serde_json::from_str(&data).map_err(|err| {
                        anyhow!(err).context("Failed to deserialize text message as JSON")
                    })
                }
                _ => {
                    return Some(Err(anyhow!("Only binary and text messages are accepted.")));
                }
            };
            Some(deserialized_res)
        })
    }
}
