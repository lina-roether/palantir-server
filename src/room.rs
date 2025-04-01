use std::collections::HashMap;

use anyhow::{anyhow, Context};
use log::error;
use tokio::{
    sync::mpsc::{self},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::session::SessionMsg;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPermissions {
    pub can_share: bool,
    pub can_close: bool,
}

impl UserPermissions {
    pub fn host() -> Self {
        Self {
            can_share: true,
            can_close: true,
        }
    }

    pub fn trusted() -> Self {
        Self {
            can_share: true,
            can_close: false,
        }
    }

    pub fn spectator() -> Self {
        Self {
            can_share: false,
            can_close: false,
        }
    }
}

impl Default for UserPermissions {
    fn default() -> Self {
        Self::spectator()
    }
}

#[derive(Debug)]
pub struct SessionInfo {
    pub session_id: Uuid,
    pub session_message_tx: mpsc::WeakSender<SessionMsg>,
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub enum RoomCloseReason {
    ClosedByHost,
    ServerError,
}

#[derive(Debug)]
enum RoomCmd {
    Join(UserPermissions, SessionInfo),
    Close(RoomCloseReason),
}

#[derive(Debug, Clone)]
struct User {
    name: String,
    permissions: UserPermissions,
    message_tx: mpsc::WeakSender<SessionMsg>,
}

#[derive(Debug, Clone)]
pub enum RoomMsg {
    RequestState,
    Leave(Uuid),
}

#[derive(Debug)]
struct RoomController {
    password: String,
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
        permissions: UserPermissions,
        info: SessionInfo,
    ) -> anyhow::Result<mpsc::WeakSender<RoomMsg>> {
        self.command_tx
            .send(RoomCmd::Join(permissions, info))
            .await?;
        Ok(self.message_sender())
    }

    async fn close(self, reason: RoomCloseReason) -> anyhow::Result<()> {
        self.command_tx.send(RoomCmd::Close(reason)).await?;
        self.join_handle.await?;
        Ok(())
    }
}

pub struct RoomHandle {
    pub id: Uuid,
    pub permissions: UserPermissions,
    pub message_tx: mpsc::WeakSender<RoomMsg>,
}

#[derive(Debug, Clone)]
pub struct UserData {
    pub id: Uuid,
    pub name: String,
    pub permissions: UserPermissions,
}

#[derive(Debug, Clone)]
pub struct RoomState {
    pub id: Uuid,
    pub name: String,
    pub password: String,
    pub users: Vec<UserData>,
}

struct Room {
    id: Uuid,
    running: bool,
    name: String,
    password: String,
    users: HashMap<Uuid, User>,
    command_rx: mpsc::Receiver<RoomCmd>,
    message_rx: mpsc::Receiver<RoomMsg>,
}

impl Room {
    fn new(
        name: String,
        password: String,
        command_rx: mpsc::Receiver<RoomCmd>,
        message_rx: mpsc::Receiver<RoomMsg>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            running: true,
            name,
            password,
            command_rx,
            message_rx,
            users: HashMap::new(),
        }
    }

    fn get_state(&self) -> RoomState {
        RoomState {
            id: self.id,
            name: self.name.clone(),
            password: self.password.clone(),
            users: self
                .users
                .iter()
                .map(|(id, user)| UserData {
                    id: *id,
                    name: user.name.clone(),
                    permissions: user.permissions.clone(),
                })
                .collect(),
        }
    }

    fn create(name: String, password: String) -> (Uuid, RoomController) {
        let (command_tx, command_rx) = mpsc::channel::<RoomCmd>(8);
        let (message_tx, message_rx) = mpsc::channel::<RoomMsg>(32);

        let mut room = Room::new(name, password.clone(), command_rx, message_rx);
        let room_id = room.id;

        let join_handle = tokio::spawn(async move { room.run().await });

        let controller = RoomController {
            password,
            command_tx,
            message_tx,
            join_handle,
        };

        (room_id, controller)
    }

    async fn send_user_msg(&mut self, id: Uuid, msg: SessionMsg) -> anyhow::Result<()> {
        let Some(user) = self.users.get(&id) else {
            return Ok(());
        };
        let Some(message_tx) = user.message_tx.upgrade() else {
            Box::pin(self.leave(id)).await;
            return Ok(());
        };
        message_tx.send(msg).await?;
        Ok(())
    }

    fn user_ids(&self) -> Vec<Uuid> {
        self.users.keys().copied().collect()
    }

    async fn broadcast_msg(&mut self, msg: SessionMsg) {
        for id in self.user_ids() {
            let result = self.send_user_msg(id, msg.clone()).await;
            if let Some(err) = result.err() {
                error!("Failed to broadcast message to user {id}: {err:?}");
            }
        }
    }

    async fn broadcast_state(&mut self) {
        self.broadcast_msg(SessionMsg::RoomState(self.get_state()))
            .await;
    }

    async fn leave(&mut self, session_id: Uuid) {
        self.users.remove(&session_id);
        if self.users.is_empty() {
            // Close the room if it has no users
            self.close(RoomCloseReason::ClosedByHost).await;
            return;
        }
        self.broadcast_state().await;
    }

    async fn handle_msg(&mut self, msg: RoomMsg) {
        match msg {
            RoomMsg::RequestState => self.broadcast_state().await,
            RoomMsg::Leave(session_id) => self.leave(session_id).await,
        }
    }

    async fn join(
        &mut self,
        permissions: UserPermissions,
        session_info: SessionInfo,
    ) -> anyhow::Result<()> {
        if self.users.contains_key(&session_info.session_id) {
            return Err(anyhow!("Already joined this room"));
        }
        self.users.insert(
            session_info.session_id,
            User {
                name: session_info.name,
                permissions,
                message_tx: session_info.session_message_tx,
            },
        );
        Ok(())
    }

    async fn close(&mut self, reason: RoomCloseReason) {
        log::debug!("Closing room {} ('{}'): {reason:?}", self.id, self.name);
        self.running = false;
        self.broadcast_msg(SessionMsg::RoomClosed(reason)).await;
    }

    async fn handle_cmd(&mut self, cmd: RoomCmd) {
        let result = match cmd {
            RoomCmd::Join(user_role, session_info) => self.join(user_role, session_info).await,
            RoomCmd::Close(reason) => {
                self.close(reason).await;
                Ok(())
            }
        };
        if let Some(err) = result.err() {
            error!("Failed to handle room command: {err:?}");
        }
    }

    async fn run(&mut self) {
        while self.running {
            tokio::select! {
                cmd = self.command_rx.recv() => {
                    if let Some(cmd) = cmd {
                        self.handle_cmd(cmd).await
                    } else {
                        error!("Room command receiver was unexpectedly closed");
                        self.close(RoomCloseReason::ServerError).await
                    }
                }
                msg = self.message_rx.recv() => {
                    if let Some(msg) = msg {
                        self.handle_msg(msg).await
                    } else {
                        error!("Room message receiver was unexpectedly closed");
                        self.close(RoomCloseReason::ServerError).await
                    }
                }
            }
        }
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

    pub async fn create_room(
        &mut self,
        name: String,
        password: String,
        session_info: SessionInfo,
    ) -> anyhow::Result<RoomHandle> {
        log::debug!(
            "Creating room with name {name} for session {}...",
            session_info.session_id
        );
        let permissions = UserPermissions::host();

        let (id, mut controller) = Room::create(name, password);
        controller
            .join(permissions.clone(), session_info)
            .await
            .context("Failed to create new room")?;
        let message_tx = controller.message_sender();
        self.room_controllers.insert(id, controller);
        Ok(RoomHandle {
            id,
            permissions,
            message_tx,
        })
    }

    pub fn get_room_password(&self, id: Uuid) -> Option<String> {
        let controller = self.room_controllers.get(&id)?;
        Some(controller.password.clone())
    }

    pub async fn join_room(
        &mut self,
        id: Uuid,
        session_info: SessionInfo,
    ) -> anyhow::Result<Option<RoomHandle>> {
        // TODO: it's probably not the best idea to assume we trust anyone who joins the room, but
        // there isn't a system for assigning permissions yet (1.4.2025)
        let permissions = UserPermissions::trusted();

        let Some(controller) = self.room_controllers.get_mut(&id) else {
            return Ok(None);
        };
        let message_tx = controller
            .join(permissions.clone(), session_info)
            .await
            .context(format!("Failed to join room {id}"))?;
        Ok(Some(RoomHandle {
            id,
            permissions,
            message_tx,
        }))
    }

    pub async fn close_room(&mut self, id: Uuid, reason: RoomCloseReason) -> anyhow::Result<()> {
        let Some(controller) = self.room_controllers.remove(&id) else {
            return Ok(());
        };
        controller
            .close(reason)
            .await
            .context(format!("Failed to close room {id}"))?;
        Ok(())
    }
}
