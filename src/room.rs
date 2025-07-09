use std::collections::HashMap;

use anyhow::{anyhow, Context};
use log::error;
use tokio::{
    sync::{mpsc, watch},
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
    playback::{Playback, PlaybackInfo, PlaybackRequest, StopReason},
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
    pub can_host: bool,
    pub can_set_roles: bool,
    pub can_kick: bool,
    pub can_close: bool,
}

impl From<UserRole> for UserPermissions {
    fn from(value: UserRole) -> Self {
        match value {
            UserRole::Host => Self {
                can_host: true,
                can_set_roles: true,
                can_kick: true,
                can_close: true,
            },
            UserRole::Guest => Self {
                can_host: true,
                can_set_roles: false,
                can_kick: false,
                can_close: false,
            },
            UserRole::Spectator => Self {
                can_host: false,
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
            can_host: value.can_host,
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

#[derive(Debug, Clone)]
pub struct User {
    pub role: UserRole,
    pub session: SessionHandle,
}

#[derive(Debug, Clone)]
pub enum RoomRequest {
    GetState,
    SetRole(SessionId, UserRole),
    Leave(SessionId),
    PlaybackHost(SessionId),
    PlaybackConnect(SessionId),
    Playback(SessionId, PlaybackRequest),
}

#[derive(Debug)]
struct RoomController {
    id: RoomId,
    name: String,
    password: String,
    command_tx: mpsc::Sender<RoomCmd>,
    request_tx: mpsc::Sender<RoomRequest>,
    result_rx: watch::Receiver<anyhow::Result<()>>,
    join_handle: JoinHandle<()>,
}

impl RoomController {
    fn handle(&self, role: UserRole) -> RoomHandle {
        RoomHandle {
            id: self.id,
            name: self.name.clone(),
            role,
            request_tx: self.request_tx.clone().downgrade(),
            result_rx: self.result_rx.clone(),
        }
    }

    async fn join(&mut self, role: UserRole, session: SessionHandle) -> anyhow::Result<RoomHandle> {
        self.command_tx.send(RoomCmd::Join(role, session)).await?;
        Ok(self.handle(role))
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
    pub name: String,
    pub role: UserRole,
    request_tx: mpsc::WeakSender<RoomRequest>,
    result_rx: watch::Receiver<anyhow::Result<()>>,
}

impl RoomHandle {
    pub async fn send_request(&mut self, req: RoomRequest) -> anyhow::Result<bool> {
        let Some(request_tx) = self.request_tx.upgrade() else {
            return Ok(false);
        };
        request_tx.send(req).await?;
        self.result_rx.changed().await?;
        if let Err(err) = &*self.result_rx.borrow_and_update() {
            // anyhow's errors aren't clonable... not ideal, but works
            return Err(anyhow!("{err:?}"));
        }

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
    request_rx: mpsc::Receiver<RoomRequest>,
    result_tx: watch::Sender<anyhow::Result<()>>,
}

impl Room {
    fn new(
        name: String,
        password: String,
        command_rx: mpsc::Receiver<RoomCmd>,
        request_rx: mpsc::Receiver<RoomRequest>,
        result_tx: watch::Sender<anyhow::Result<()>>,
    ) -> Self {
        Self {
            id: RoomId::new(),
            running: true,
            name,
            password,
            command_rx,
            request_rx,
            result_tx,
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

    fn create(name: String, password: String) -> RoomController {
        let (command_tx, command_rx) = mpsc::channel::<RoomCmd>(8);
        let (request_tx, request_rx) = mpsc::channel::<RoomRequest>(32);
        let (result_tx, result_rx) = watch::channel::<anyhow::Result<()>>(Ok(()));

        let mut room = Room::new(
            name.clone(),
            password.clone(),
            command_rx,
            request_rx,
            result_tx,
        );
        let room_id = room.id;

        let join_handle = tokio::spawn(async move { room.run().await });

        let controller = RoomController {
            id: room_id,
            name,
            password,
            command_tx,
            request_tx,
            result_rx,
            join_handle,
        };

        controller
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

    async fn broadcast_msg(&mut self, msg: SessionMsg) -> anyhow::Result<()> {
        let mut result = Ok(());
        for id in self.user_ids() {
            if let Err(err) = self.send_user_msg(id, msg.clone()).await {
                error!("Failed to broadcast message to user {id}: {err:?}");
                if result.is_ok() {
                    result = Err(anyhow!("Failed to broadcast message to one or more users"))
                }
            }
        }
        result
    }

    async fn broadcast_state(&mut self) -> anyhow::Result<()> {
        self.broadcast_msg(SessionMsg::RoomState(self.get_state()))
            .await
    }

    async fn leave(&mut self, session_id: SessionId) {
        let Some(user) = self.users.remove(&session_id) else {
            return;
        };
        if self.users.is_empty() {
            // Close the room if it has no users
            if let Err(err) = self.close(RoomCloseReason::ClosedByHost).await {
                log::error!("Error while closing empty room: {err:?}");
            }
            return;
        }
        if self
            .users
            .iter()
            .all(|(_, user)| user.role != UserRole::Host)
        {
            let Some(new_host_id) = self.choose_new_host_id() else {
                log::error!(
                    "Failed to choose a new host id in session {session_id}! closing the room!"
                );
                let _ = self.close(RoomCloseReason::ServerError).await;
                return;
            };
            if let Err(err) = self.set_role(UserRole::Host, new_host_id).await {
                log::error!("Failed to set new room host: {err:?}");
                let _ = self.close(RoomCloseReason::ServerError).await;
            }
        }
        log::info!("User '{}' left room '{}'", user.session.name, self.name);
        if let Err(err) = self.broadcast_state().await {
            log::error!("Failed to broadcast state after leaving the room: {err}");
        }
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

    async fn host_playback(&mut self, session_id: SessionId) -> anyhow::Result<()> {
        if let Some(mut playback) = self.playback.take() {
            if let Err(err) = playback.stop(StopReason::Superseded).await {
                log::error!("Failed to stop existing playback: {err}");
            }
        }

        let Some(host) = self.users.get(&session_id) else {
            return Err(anyhow!("Unknown user"));
        };

        self.playback = Some(Playback::new(host.session.clone()));

        log::info!(
            "User '{}' is hosting playback in room '{}'",
            host.session.name,
            self.name
        );

        self.send_user_msg(host.session.id, SessionMsg::PlaybackHosting)
            .await?;

        Ok(())
    }

    async fn connect_playback(&mut self, session_id: SessionId) -> anyhow::Result<()> {
        let Some(playback) = &mut self.playback else {
            return Err(anyhow!("No active playback"));
        };

        let Some(subscriber) = self.users.get(&session_id) else {
            return Err(anyhow!("Unkown user"));
        };

        playback.connect(subscriber.session.clone()).await?;

        Ok(())
    }

    async fn playback_request(
        &mut self,
        session_id: SessionId,
        request: PlaybackRequest,
    ) -> anyhow::Result<()> {
        let Some(playback) = &mut self.playback else {
            return Err(anyhow!("No active playback"));
        };

        playback.handle_request(session_id, request).await
    }

    async fn handle_request(&mut self, request: RoomRequest) {
        let result = match request {
            RoomRequest::GetState => self.broadcast_state().await,
            RoomRequest::SetRole(session_id, role) => self.set_role(role, session_id).await,
            RoomRequest::Leave(session_id) => {
                self.leave(session_id).await;
                Ok(())
            }
            RoomRequest::PlaybackHost(session_id) => self.host_playback(session_id).await,
            RoomRequest::PlaybackConnect(session_id) => self.connect_playback(session_id).await,
            RoomRequest::Playback(session_id, request) => {
                self.playback_request(session_id, request).await
            }
        };
        if let Err(err) = self.result_tx.send(result) {
            log::error!("Failed to send room request result: {err:?}");
        }
    }

    async fn join(&mut self, role: UserRole, session: SessionHandle) -> anyhow::Result<()> {
        if self.users.contains_key(&session.id) {
            return Err(anyhow!("Already joined this room"));
        }
        log::info!("User '{}' has joined room '{}'", session.name, self.name);
        self.users.insert(session.id, User { role, session });
        self.broadcast_state().await
    }

    async fn set_role(&mut self, role: UserRole, session_id: SessionId) -> anyhow::Result<()> {
        let Some(user) = self.users.get_mut(&session_id) else {
            return Ok(());
        };
        user.role = role;
        log::info!("Setting rome of user '{}' to {role:?}", user.session.name);
        self.broadcast_state().await
    }

    async fn close(&mut self, reason: RoomCloseReason) -> anyhow::Result<()> {
        log::debug!("Closing room {} ('{}'): {reason:?}", self.id, self.name);
        self.running = false;
        log::info!("Room '{}' has been closed", self.name);
        self.broadcast_msg(SessionMsg::RoomClosed(reason)).await
    }

    async fn handle_cmd(&mut self, cmd: RoomCmd) {
        let result = match cmd {
            RoomCmd::Join(user_role, session_info) => self.join(user_role, session_info).await,
            RoomCmd::Close(reason) => self.close(reason).await,
        };
        if let Err(err) = self.result_tx.send(result) {
            error!("Failed to send room command result: {err:?}");
        }
    }

    async fn run(&mut self) {
        log::info!("Room '{}' created", self.name);
        while self.running {
            tokio::select! {
                cmd = self.command_rx.recv() => {
                    if let Some(cmd) = cmd {
                        self.handle_cmd(cmd).await
                    } else {
                        error!("Room command receiver was unexpectedly closed");
                        let _ = self.close(RoomCloseReason::ServerError).await;
                    }
                }
                req = self.request_rx.recv() => {
                    if let Some(req) = req {
                        self.handle_request(req).await
                    } else {
                        error!("Room request receiver was unexpectedly closed");
                        let _ = self.close(RoomCloseReason::ServerError).await;
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

        let mut controller = Room::create(name, password);
        controller
            .join(role, session)
            .await
            .context("Failed to create new room")?;
        let handle = controller.handle(role);
        self.room_controllers.insert(controller.id, controller);
        Ok(handle)
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
        let handle = controller
            .join(role, session)
            .await
            .context(format!("Failed to join room {id}"))?;
        Ok(Some(handle))
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
