use std::sync::Arc;

use anyhow::{anyhow, Context};
use log::{error, warn};
use tokio::sync::{self, mpsc};
use uuid::Uuid;

use crate::{
    api_access::ApiPermissions,
    connection::Connection,
    messages::{Message, MessageBody, RoomDisconnectedMsgBodyV1, RoomDisconnectedReasonV1},
    room::{self, RoomCloseReason, RoomHandle, RoomManager, RoomMsg, RoomState, UserRole},
};

#[derive(Debug, Clone)]
pub enum SessionMsg {
    RoomState(RoomState),
    RoomClosed(RoomCloseReason),
}

pub struct Session {
    id: Uuid,
    name: String,
    api_permissions: ApiPermissions,
    room_manager: Arc<sync::Mutex<RoomManager>>,
    room: Option<RoomHandle>,
    message_tx: mpsc::Sender<SessionMsg>,
    message_rx: mpsc::Receiver<SessionMsg>,
    connection: Connection,
}

impl Session {
    pub fn new(
        name: String,
        connection: Connection,
        api_permissions: ApiPermissions,
        room_manager: Arc<sync::Mutex<RoomManager>>,
    ) -> Self {
        let (message_tx, message_rx) = mpsc::channel::<SessionMsg>(32);
        Self {
            id: Uuid::new_v4(),
            api_permissions,
            name,
            room: None,
            message_rx,
            message_tx,
            connection,
            room_manager,
        }
    }

    pub async fn run(&mut self) {
        todo!()
    }

    async fn create_room(&mut self, name: String, password: String) -> anyhow::Result<()> {
        if !self.api_permissions.host {
            return Err(anyhow!("Your account is not permitted to host rooms"));
        }

        self.leave_room()
            .await
            .context("Failed to leave current room before opening a new one")?;

        let room_handle = self
            .room_manager
            .lock()
            .await
            .create_room(name, password, self.session_info())
            .await?;
        self.room = Some(room_handle);

        self.connection
            .send(Message::new(MessageBody::RoomCreateAckV1))
            .await
            .context("Failed to send ACK message")?;

        Ok(())
    }

    async fn close_room(&mut self) -> anyhow::Result<()> {
        let Some(room_handle) = &self.room else {
            return Ok(());
        };

        if room_handle.role != UserRole::Host {
            return Err(anyhow!("Not authorized to close the room"));
        }

        self.room_manager
            .lock()
            .await
            .close_room(room_handle.id, RoomCloseReason::ClosedByHost)
            .await?;
        self.room = None;

        self.connection
            .send(Message::new(MessageBody::RoomCloseAckV1))
            .await
            .context("Failed to send ACK message")?;

        Ok(())
    }

    async fn join_room(&mut self, room_id: Uuid, password: String) -> anyhow::Result<()> {
        self.leave_room()
            .await
            .context("Failed to leave current room before joining a new one")?;

        let mut room_mgr = self.room_manager.lock().await;

        if Some(password) != room_mgr.get_room_password(room_id) {
            return Err(anyhow!("Incorrect password"));
        }

        let room_handle = room_mgr.join_room(room_id, self.session_info()).await?;

        if let Some(handle) = room_handle {
            self.room = Some(handle);
            self.connection
                .send(Message::new(MessageBody::RoomJoinAckV1))
                .await
                .context("Failed to send ACK message")?;
        } else {
            self.connection
                .send_error(format!("Room {room_id} does not exist"))
                .await;
        }

        Ok(())
    }

    async fn leave_room(&mut self) -> anyhow::Result<()> {
        self.send_room_msg(RoomMsg::Leave(self.id)).await?;
        self.room = None;
        self.connection
            .send(Message::new(MessageBody::RoomLeaveAckV1))
            .await
            .context("Failed to send ACK message")?;
        Ok(())
    }

    async fn send_room_msg(&mut self, msg: RoomMsg) -> anyhow::Result<()> {
        let Some(room_handle) = &self.room else {
            return Ok(());
        };
        let Some(message_tx) = room_handle.message_tx.upgrade() else {
            warn!("Room {} was unexpectedly closed", room_handle.id);
            self.room = None;
            self.connection
                .send(Message::new(MessageBody::RoomDisconnectedV1(
                    RoomDisconnectedMsgBodyV1 {
                        reason: RoomDisconnectedReasonV1::ServerError,
                        message: "The room crashed! sowwy ;-;".to_string(),
                    },
                )))
                .await
                .context("Failed to send disconnect message")?;
            return Ok(());
        };
        message_tx.send(msg).await?;
        Ok(())
    }

    async fn request_state(&mut self) -> anyhow::Result<()> {
        self.send_room_msg(RoomMsg::RequestState).await
    }

    async fn handle_client_msg(&mut self, msg: Message) {
        let result = match msg.body {
            MessageBody::RoomCreateV1(body) => self.create_room(body.name, body.password).await,
            MessageBody::RoomCloseV1 => self.close_room().await,
            MessageBody::RoomJoinV1(body) => self.join_room(body.id, body.password).await,
            MessageBody::RoomLeaveV1 => self.leave_room().await,
            MessageBody::RoomRequestStateV1 => self.request_state().await,
            _ => Ok(()),
        };
        if let Some(err) = result.err() {
            error!("{err:?}");
            self.connection.send_error(err).await;
        }
    }

    fn session_info(&self) -> room::SessionInfo {
        room::SessionInfo {
            session_id: self.id,
            session_message_tx: self.message_tx.clone().downgrade(),
            name: self.name.clone(),
        }
    }
}
