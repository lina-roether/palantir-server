use std::{error::Error, time::SystemTime};

use anyhow::{anyhow, Context as _};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

use crate::utils::timestamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionLoginMsgBodyV1 {
    pub username: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionClosedReasonV1 {
    #[serde(rename = "unauthorized")]
    Unauthorized,

    #[serde(rename = "server_error")]
    ServerError,

    #[serde(rename = "room_closed")]
    RoomClosed,

    #[serde(rename = "timeout")]
    Timeout,

    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionClosedMsgBodyV1 {
    pub reason: ConnectionClosedReasonV1,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionClientErrorMsgBodyV1 {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomCreateMsgBodyV1 {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomJoinMsgBodyV1 {
    pub id: Uuid,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomUserRoleV1 {
    #[serde(rename = "host")]
    Host,

    #[serde(rename = "guest")]
    Guest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomUserV1 {
    pub id: Uuid,
    pub name: String,
    pub role: RoomUserRoleV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomStateMsgBodyV1 {
    pub id: Uuid,
    pub name: String,
    pub password: String,
    pub users: Vec<RoomUserV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomDisconnectedReasonV1 {
    #[serde(rename = "closed_by_host")]
    ClosedByHost,

    #[serde(rename = "unauthorized")]
    Unauthorized,

    #[serde(rename = "server_error")]
    ServerError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomDisconnectedMsgBodyV1 {
    pub reason: RoomDisconnectedReasonV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackSelectMsgBodyV1 {
    pub page_href: String,
    pub frame_href: String,
    pub element_query: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackSyncMsgBodyV1 {
    pub active_sync: bool,
    pub playing: bool,
    pub time: u64,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "m")]
#[non_exhaustive]
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

    #[serde(rename = "connection::keepalive/v1")]
    ConnectionKeepaliveV1,

    #[serde(rename = "room::create/v1")]
    RoomCreateV1(RoomCreateMsgBodyV1),

    #[serde(rename = "room::create_ack/v1")]
    RoomCreateAckV1,

    #[serde(rename = "room::close/v1")]
    RoomCloseV1,

    #[serde(rename = "room::close_ack/v1")]
    RoomCloseAckV1,

    #[serde(rename = "room::join/v1")]
    RoomJoinV1(RoomJoinMsgBodyV1),

    #[serde(rename = "room::join_ack/v1")]
    RoomJoinAckV1,

    #[serde(rename = "room::leave/v1")]
    RoomLeaveV1,

    #[serde(rename = "room::leave_ack/v1")]
    RoomLeaveAckV1,

    #[serde(rename = "room::disconnected/v1")]
    RoomDisconnectedV1(RoomDisconnectedMsgBodyV1),

    #[serde(rename = "room::request_state/v1")]
    RoomRequestStateV1,

    #[serde(rename = "room::state/v1")]
    RoomStateV1(RoomStateMsgBodyV1),

    #[serde(rename = "playback::select/v1")]
    PlaybackSelectV1(PlaybackSelectMsgBodyV1),

    #[serde(rename = "playback::sync/v1")]
    PlaybackSyncV1(PlaybackSyncMsgBodyV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "t")]
    pub timestamp: u64,

    #[serde(flatten)]
    pub body: MessageBody,
}

impl Message {
    pub fn new(body: MessageBody) -> Self {
        Self::new_with_timestamp(body, timestamp())
    }

    pub fn new_with_timestamp(body: MessageBody, timestamp: u64) -> Self {
        Self { body, timestamp }
    }
}

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
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

impl<S> MessageChannel<S>
where
    S: Sink<tungstenite::Message> + Unpin,
    S::Error: Error + Send + Sync + 'static,
{
    pub async fn send(&mut self, message: Message) -> Result<(), anyhow::Error> {
        log::debug!("Sending message {message:?}");
        let serialized_msg = match self.format {
            MessageFormat::Msgpack => tungstenite::Message::Binary(
                rmp_serde::to_vec(&message).context("Failed to serialize message as MsgPack")?,
            ),
            MessageFormat::Json => tungstenite::Message::Text(
                serde_json::to_string(&message).context("Failed to serialize message as JSON")?,
            ),
        };
        self.ws
            .send(serialized_msg)
            .await
            .map_err(anyhow::Error::from)
    }

    pub async fn close(&mut self) -> Result<(), anyhow::Error> {
        self.ws.close().await?;
        Ok(())
    }
}

impl<S> MessageChannel<S>
where
    S: Stream<Item = tungstenite::Result<tungstenite::Message>> + Unpin,
{
    pub async fn recv(&mut self) -> Option<Result<Message, anyhow::Error>> {
        let msg = match self.ws.next().await? {
            Ok(msg) => msg,
            Err(err) => return Some(Err(anyhow!(err))),
        };
        let deserialized_msg: anyhow::Result<Message> = match msg {
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
            tungstenite::Message::Close(frame) => {
                log::debug!("Received close frame: {frame:?}");
                return None;
            }
            _ => return Some(Err(anyhow!("Only binary and text messages are accepted."))),
        };
        log::debug!("Received message {deserialized_msg:?}");
        Some(deserialized_msg)
    }
}

#[cfg(test)]
mod tests {
    use futures::stream;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn should_send_message() {
        // given
        let mut messages = Vec::new();
        let mut channel = MessageChannel::new(&mut messages);

        // when
        channel
            .send(Message::new_with_timestamp(
                MessageBody::ConnectionPingV1,
                69420,
            ))
            .await
            .unwrap();

        // then
        assert_eq!(messages.len(), 1);
        let tungstenite::Message::Binary(data_recieved) = &messages[0] else {
            panic!("Data received should be binary");
        };
        let obj_received: serde_json::Value = rmp_serde::from_slice(data_recieved).unwrap();

        let obj_expected = json!({
            "t": 69420,
            "m": "connection::ping/v1",
        });
        assert_eq!(obj_received, obj_expected);
    }

    #[tokio::test]
    async fn should_receive_message() {
        // given
        let messages = vec![tungstenite::Result::Ok(tungstenite::Message::binary(
            rmp_serde::to_vec(&json!({
                "t": 42069,
                "m": "connection::pong/v1"
            }))
            .unwrap(),
        ))];
        let mut channel = MessageChannel::new(stream::iter(messages));

        // when
        let msg = channel.recv().await.unwrap().unwrap();

        // then
        assert_eq!(
            msg,
            Message::new_with_timestamp(MessageBody::ConnectionPongV1, 42069)
        );
        assert!(channel.recv().await.is_none());
    }

    #[tokio::test]
    async fn should_handle_malformed_messages() {
        // given
        let messages = vec![tungstenite::Result::Ok(tungstenite::Message::binary(
            rmp_serde::to_vec(&json!({
                "t": 42069,
                "m": "AcddafsdfSfFdasdsadDDFSFÖDSFD"
            }))
            .unwrap(),
        ))];
        let mut channel = MessageChannel::new(stream::iter(messages));

        // when
        let result = channel.recv().await.unwrap();

        // then
        assert!(result.is_err());
        assert!(channel.recv().await.is_none());
    }
}
