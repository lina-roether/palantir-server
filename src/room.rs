use std::collections::HashMap;

use anyhow::{anyhow, Context};
use log::error;
use tokio::{
    sync::mpsc::{self},
    task::JoinHandle,
};

id_type!(RoomId);

impl From<dto::RoomIdV1> for RoomId {
    fn from(value: dto::RoomIdV1) -> Self {
        Self::from(*value)
    }
}

impl From<RoomId> for dto::RoomIdV1 {
    fn from(value: RoomId) -> Self {
        Self::from(*value)
    }
}

use crate::{
    id_type,
    messages::dto,
    playback::{Playback, PlaybackInfo, PlaybackSource, PlaybackState},
    session::{SessionHandle, SessionId, SessionMsg},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRole {
    Host,
    Guest,
    Spectator,
}

impl UserRole {
    pub fn permissions(self) -> UserPermissions {
        UserPermissions::from(self)
    }
}

impl From<dto::RoomUserRoleV1> for UserRole {
    fn from(value: dto::RoomUserRoleV1) -> Self {
        match value {
            dto::RoomUserRoleV1::Host => UserRole::Host,
            dto::RoomUserRoleV1::Guest => UserRole::Guest,
            dto::RoomUserRoleV1::Spectator => UserRole::Spectator,
        }
    }
}

impl From<UserRole> for dto::RoomUserRoleV1 {
    fn from(value: UserRole) -> Self {
        match value {
            UserRole::Host => dto::RoomUserRoleV1::Host,
            UserRole::Guest => dto::RoomUserRoleV1::Guest,
            UserRole::Spectator => dto::RoomUserRoleV1::Spectator,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPermissions {
    pub can_share: bool,
    pub can_set_roles: bool,
    pub can_kick: bool,
    pub can_close: bool,
}

impl From<UserRole> for UserPermissions {
    fn from(value: UserRole) -> Self {
        match value {
            UserRole::Host => Self {
                can_share: true,
                can_set_roles: true,
                can_kick: true,
                can_close: true,
            },
            UserRole::Guest => Self {
                can_share: true,
                can_set_roles: false,
                can_kick: false,
                can_close: false,
            },
            UserRole::Spectator => Self {
                can_share: false,
                can_set_roles: false,
                can_kick: false,
                can_close: false,
            },
        }
    }
}

impl From<UserPermissions> for dto::RoomUserPermissionsV1 {
    fn from(value: UserPermissions) -> Self {
        Self {
            can_close: value.can_close,
            can_share: value.can_share,
            can_set_roles: value.can_set_roles,
            can_kick: value.can_kick,
        }
    }
}

impl Default for UserPermissions {
    fn default() -> Self {
        Self::from(UserRole::Spectator)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RoomCloseReason {
    ClosedByHost,
    ServerError,
}

#[derive(Debug)]
enum RoomCmd {
    Join(UserRole, SessionHandle),
    Close(RoomCloseReason),
}

#[derive(Debug)]
struct User {
    role: UserRole,
    session: SessionHandle,
}

#[derive(Debug, Clone)]
pub enum RoomMsg {
    RequestState,
    SetRole(SessionId, UserRole),
    PlaybackStart(SessionId, PlaybackSource),
    PlaybackStop(SessionId),
    PlaybackConnect(SessionId),
    PlaybackDisconnect(SessionId),
    PlaybackSync(PlaybackState),
    Leave(SessionId),
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
        role: UserRole,
        session: SessionHandle,
    ) -> anyhow::Result<mpsc::WeakSender<RoomMsg>> {
        self.command_tx.send(RoomCmd::Join(role, session)).await?;
        Ok(self.message_sender())
    }

    async fn close(self, reason: RoomCloseReason) -> anyhow::Result<()> {
        self.command_tx.send(RoomCmd::Close(reason)).await?;
        self.join_handle.await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct RoomHandle {
    pub id: RoomId,
    pub role: UserRole,
    message_tx: mpsc::WeakSender<RoomMsg>,
}

impl RoomHandle {
    pub async fn send_message(&self, msg: RoomMsg) -> anyhow::Result<bool> {
        let Some(message_tx) = self.message_tx.upgrade() else {
            return Ok(false);
        };
        message_tx.send(msg).await?;
        Ok(true)
    }
}

#[derive(Debug, Clone)]
pub struct UserData {
    pub id: SessionId,
    pub name: String,
    pub role: UserRole,
}

impl From<UserData> for dto::RoomUserV1 {
    fn from(value: UserData) -> Self {
        Self {
            id: value.id.into(),
            name: value.name,
            role: value.role.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoomState {
    pub id: RoomId,
    pub name: String,
    pub password: String,
    pub playback_info: Option<PlaybackInfo>,
    pub users: Vec<UserData>,
}

impl From<RoomState> for dto::RoomStateMsgBodyV1 {
    fn from(value: RoomState) -> Self {
        Self {
            id: value.id.into(),
            name: value.name,
            password: value.password,
            users: value.users.into_iter().map(From::from).collect(),
            playback_info: value.playback_info.map(From::from),
        }
    }
}

struct Room {
    id: RoomId,
    running: bool,
    name: String,
    password: String,
    users: HashMap<SessionId, User>,
    playback: Option<Playback>,
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
            id: RoomId::new(),
            running: true,
            name,
            password,
            command_rx,
            message_rx,
            playback: None,
            users: HashMap::new(),
        }
    }

    fn get_state(&self) -> RoomState {
        RoomState {
            id: self.id,
            name: self.name.clone(),
            password: self.password.clone(),
            playback_info: self.playback.as_ref().map(Playback::get_info),
            users: self
                .users
                .iter()
                .map(|(id, user)| UserData {
                    id: *id,
                    name: user.session.name.clone(),
                    role: user.role,
                })
                .collect(),
        }
    }

    fn create(name: String, password: String) -> (RoomId, RoomController) {
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

    async fn send_user_msg(&mut self, id: SessionId, msg: SessionMsg) -> anyhow::Result<()> {
        let Some(user) = self.users.get(&id) else {
            return Ok(());
        };
        if !user.session.send_message(msg).await? {
            Box::pin(self.leave(id)).await;
        };
        Ok(())
    }

    fn user_ids(&self) -> Vec<SessionId> {
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

    async fn leave(&mut self, session_id: SessionId) {
        self.users.remove(&session_id);
        if self.users.is_empty() {
            // Close the room if it has no users
            self.close(RoomCloseReason::ClosedByHost).await;
            return;
        }
        if self
            .users
            .iter()
            .all(|(_, user)| user.role != UserRole::Host)
        {
            let Some(new_host_id) = self.choose_new_host_id() else {
                log::debug!(
                    "failed to choose a new host id in session {session_id}! closing the room!"
                );
                self.close(RoomCloseReason::ServerError).await;
                return;
            };
            self.set_role(UserRole::Host, new_host_id).await;
        }
        self.broadcast_state().await;
    }

    fn choose_new_host_id(&mut self) -> Option<SessionId> {
        let mut new_host_id: Option<SessionId> = None;
        for (id, user) in &self.users {
            if matches!(user.role, UserRole::Host | UserRole::Guest) {
                return Some(*id);
            }
            new_host_id.get_or_insert(*id);
        }
        new_host_id
    }

    async fn start_playback(&mut self, session_id: SessionId, source: PlaybackSource) {
        todo!()
    }

    async fn handle_msg(&mut self, msg: RoomMsg) {
        match msg {
            RoomMsg::RequestState => self.broadcast_state().await,
            RoomMsg::SetRole(session_id, role) => self.set_role(role, session_id).await,
            RoomMsg::Leave(session_id) => self.leave(session_id).await,
            RoomMsg::PlaybackStart(session_id, source) => {
                self.start_playback(session_id, source).await
            }
            _ => todo!(),
        }
    }

    async fn join(&mut self, role: UserRole, session: SessionHandle) -> anyhow::Result<()> {
        if self.users.contains_key(&session.id) {
            return Err(anyhow!("Already joined this room"));
        }
        self.users.insert(session.id, User { role, session });
        self.broadcast_state().await;
        Ok(())
    }

    async fn set_role(&mut self, role: UserRole, session_id: SessionId) {
        let Some(user) = self.users.get_mut(&session_id) else {
            return;
        };
        user.role = role;
        self.broadcast_state().await;
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
    room_controllers: HashMap<RoomId, RoomController>,
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
        session: SessionHandle,
    ) -> anyhow::Result<RoomHandle> {
        log::debug!(
            "Creating room with name {name} for session {}...",
            session.id
        );
        let role = UserRole::Host;

        let (id, mut controller) = Room::create(name, password);
        controller
            .join(role, session)
            .await
            .context("Failed to create new room")?;
        let message_tx = controller.message_sender();
        self.room_controllers.insert(id, controller);
        Ok(RoomHandle {
            id,
            role,
            message_tx,
        })
    }

    pub fn get_room_password(&self, id: RoomId) -> Option<String> {
        let controller = self.room_controllers.get(&id)?;
        Some(controller.password.clone())
    }

    pub async fn join_room(
        &mut self,
        id: RoomId,
        session: SessionHandle,
    ) -> anyhow::Result<Option<RoomHandle>> {
        // TODO: it's probably not the best idea to assume we trust anyone who joins the room, but
        // there isn't a system for assigning permissions yet (1.4.2025)
        let role = UserRole::Guest;

        let Some(controller) = self.room_controllers.get_mut(&id) else {
            return Ok(None);
        };
        let message_tx = controller
            .join(role, session)
            .await
            .context(format!("Failed to join room {id}"))?;
        Ok(Some(RoomHandle {
            id,
            role,
            message_tx,
        }))
    }

    pub async fn close_room(&mut self, id: RoomId, reason: RoomCloseReason) -> anyhow::Result<()> {
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
