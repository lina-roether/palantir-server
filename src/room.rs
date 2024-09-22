use std::collections::HashMap;

use anyhow::Context;
use parking_lot::{Mutex, RwLock};
use tokio::{
    sync::mpsc::{self, WeakSender},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::session::SessionMsg;

pub enum UserRole {
    Host,
    Guest,
}

pub struct SessionInfo {
    pub session_id: Uuid,
    pub session_message_tx: mpsc::WeakSender<SessionMsg>,
    pub name: String,
}

enum RoomCmd {
    Join(UserRole, SessionInfo),
    Close,
}

struct User {
    name: String,
    role: UserRole,
    message_tx: mpsc::WeakSender<SessionMsg>,
}

pub enum RoomMsg {
    Leave(Uuid),
}

struct RoomController {
    command_tx: mpsc::Sender<RoomCmd>,
    message_tx: mpsc::Sender<RoomMsg>,
    join_handle: JoinHandle<()>,
}

impl RoomController {
    fn message_sender(&self) -> mpsc::WeakSender<RoomMsg> {
        self.message_tx.clone().downgrade()
    }

    async fn join(
        &mut self,
        role: UserRole,
        info: SessionInfo,
    ) -> anyhow::Result<mpsc::WeakSender<RoomMsg>> {
        self.command_tx.send(RoomCmd::Join(role, info)).await?;
        Ok(self.message_sender())
    }

    async fn close(self) -> anyhow::Result<()> {
        self.command_tx.send(RoomCmd::Close).await?;
        self.join_handle.await?;
        Ok(())
    }
}

pub struct RoomHandle {
    pub id: Uuid,
    pub message_tx: mpsc::WeakSender<RoomMsg>,
}

struct Room {
    users: HashMap<Uuid, User>,
    command_rx: mpsc::Receiver<RoomCmd>,
    message_rx: mpsc::Receiver<RoomMsg>,
}

impl Room {
    fn new(command_rx: mpsc::Receiver<RoomCmd>, message_rx: mpsc::Receiver<RoomMsg>) -> Self {
        Self {
            command_rx,
            message_rx,
            users: HashMap::new(),
        }
    }

    fn create() -> RoomController {
        let (command_tx, command_rx) = mpsc::channel::<RoomCmd>(8);
        let (message_tx, message_rx) = mpsc::channel::<RoomMsg>(32);
        let mut room = Room::new(command_rx, message_rx);
        let join_handle = tokio::spawn(async move { room.run().await });
        RoomController {
            command_tx,
            message_tx,
            join_handle,
        }
    }

    async fn run(&mut self) {
        todo!()
    }
}

pub struct RoomManager {
    room_controllers: HashMap<Uuid, RoomController>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            room_controllers: HashMap::new(),
        }
    }

    pub async fn create_room(&mut self, session_info: SessionInfo) -> anyhow::Result<RoomHandle> {
        let id = Uuid::new_v4();
        let mut controller = Room::create();
        controller
            .join(UserRole::Host, session_info)
            .await
            .context("Failed to create new room")?;
        let message_tx = controller.message_sender();
        self.room_controllers.insert(id, controller);
        Ok(RoomHandle { id, message_tx })
    }

    pub async fn join_room(
        &mut self,
        id: Uuid,
        session_info: SessionInfo,
    ) -> anyhow::Result<Option<RoomHandle>> {
        let Some(controller) = self.room_controllers.get_mut(&id) else {
            return Ok(None);
        };
        let message_tx = controller
            .join(UserRole::Guest, session_info)
            .await
            .context(format!("Failed to join room {id}"))?;
        Ok(Some(RoomHandle { id, message_tx }))
    }

    pub async fn close_room(&mut self, id: Uuid) -> anyhow::Result<()> {
        let Some(controller) = self.room_controllers.remove(&id) else {
            return Ok(());
        };
        controller
            .close()
            .await
            .context(format!("Failed to close room {id}"))?;
        Ok(())
    }
}
