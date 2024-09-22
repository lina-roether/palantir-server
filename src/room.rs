use std::collections::HashMap;

use anyhow::{anyhow, Context};
use log::error;
use tokio::{
    sync::mpsc::{self},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::session::SessionMsg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRole {
    Host,
    Guest,
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
    Join(UserRole, SessionInfo),
    Close(RoomCloseReason),
}

struct User {
    name: String,
    role: UserRole,
    message_tx: mpsc::WeakSender<SessionMsg>,
}

pub enum RoomMsg {
    RequestState,
    Leave(Uuid),
}

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

    fn authenticate(&self, password: String) -> anyhow::Result<()> {
        if password != self.password {
            return Err(anyhow!("Incorrect password!"));
        }
        Ok(())
    }

    async fn join(
        &mut self,
        role: UserRole,
        info: SessionInfo,
    ) -> anyhow::Result<mpsc::WeakSender<RoomMsg>> {
        self.command_tx.send(RoomCmd::Join(role, info)).await?;
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
    pub role: UserRole,
    pub message_tx: mpsc::WeakSender<RoomMsg>,
}

#[derive(Debug, Clone)]
pub struct UserData {
    pub id: Uuid,
    pub name: String,
    pub role: UserRole,
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
                    role: user.role,
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
            self.users.remove(&id);
            Box::pin(self.broadcast_state()).await;
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
        self.broadcast_msg(SessionMsg::RoomState(self.get_state()));
    }

    async fn leave(&mut self, session_id: Uuid) {
        self.users.remove(&session_id);
        self.broadcast_state().await;
    }

    async fn handle_msg(&mut self, msg: RoomMsg) {
        match msg {
            RoomMsg::RequestState => self.broadcast_state().await,
            RoomMsg::Leave(session_id) => self.leave(session_id).await,
        }
    }

    async fn join(&mut self, user_role: UserRole, session_info: SessionInfo) -> anyhow::Result<()> {
        if self.users.contains_key(&session_info.session_id) {
            return Err(anyhow!("Already joined this room"));
        }
        self.users.insert(
            session_info.session_id,
            User {
                name: session_info.name,
                role: user_role,
                message_tx: session_info.session_message_tx,
            },
        );
        Ok(())
    }

    async fn close(&mut self, reason: RoomCloseReason) {
        self.running = false;
        self.broadcast_msg(SessionMsg::RoomClosed(reason));
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
        let (id, mut controller) = Room::create(name, password);
        controller
            .join(UserRole::Host, session_info)
            .await
            .context("Failed to create new room")?;
        let message_tx = controller.message_sender();
        self.room_controllers.insert(id, controller);
        Ok(RoomHandle {
            id,
            role: UserRole::Host,
            message_tx,
        })
    }

    pub fn get_room_password(&self, id: Uuid) -> Option<String> {
        let Some(controller) = self.room_controllers.get(&id) else {
            return None;
        };
        Some(controller.password.clone())
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
        Ok(Some(RoomHandle {
            id,
            role: UserRole::Guest,
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
