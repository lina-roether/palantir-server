use std::sync::Arc;

use anyhow::Context;
use log::error;
use tokio::sync::{self, mpsc};
use uuid::Uuid;

use crate::{
    connection::Connection,
    messages::{Message, MessageBody},
    room::{self, RoomHandle, RoomManager, RoomMsg},
};

pub enum SessionMsg {}

pub struct Session {
    id: Uuid,
    name: String,
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
        room_manager: Arc<sync::Mutex<RoomManager>>,
    ) -> Self {
        let (message_tx, message_rx) = mpsc::channel::<SessionMsg>(32);
        Self {
            id: Uuid::new_v4(),
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

    async fn leave_room(&mut self) -> anyhow::Result<()> {
        let Some(room_handle) = self.room.take() else {
            return Ok(());
        };
        let Some(sender) = room_handle.message_tx.upgrade() else {
            return Ok(());
        };
        sender
            .send(RoomMsg::Leave(self.id))
            .await
            .context("Failed to send RoomMsg")?;
        Ok(())
    }

    async fn create_room(&mut self) -> anyhow::Result<()> {
        self.leave_room()
            .await
            .context("Failed to leave current room before opening a new one")?;

        let room_handle = self
            .room_manager
            .lock()
            .await
            .create_room(self.session_info())
            .await
            .context("Failed to create room using room manager")?;
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
        self.room_manager
            .lock()
            .await
            .close_room(room_handle.id)
            .await
            .context("Failed to close room using room manager")?;
        self.room = None;

        self.connection
            .send(Message::new(MessageBody::RoomCloseAckV1))
            .await
            .context("Failed to send ACK message")?;

        Ok(())
    }

    async fn handle_client_msg(&mut self, msg: Message) {
        let err = match msg.body {
            MessageBody::RoomCreateV1(body) => self
                .create_room()
                .await
                .context(format!("Failed to create room {}", body.name))
                .err(),
            MessageBody::RoomCloseV1 => self
                .close_room()
                .await
                .context("Failed to close room")
                .err(),
            _ => None,
        };
        if let Some(err) = err {
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
