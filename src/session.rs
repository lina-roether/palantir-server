use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Weak,
    },
    time::Duration,
};

use anyhow::{anyhow, Context};
use tokio::{
    sync::{self, mpsc},
    time,
};

id_type!(SessionId);

impl From<dto::UserIdV1> for SessionId {
    fn from(value: dto::UserIdV1) -> Self {
        Self::from(*value)
    }
}

impl From<SessionId> for dto::UserIdV1 {
    fn from(value: SessionId) -> Self {
        Self::from(*value)
    }
}

use crate::{
    connection::{CloseReason, Connection},
    id_type,
    messages::{dto, Message, MessageBody},
    playback::{DisconnectReason, PlaybackInfo, PlaybackRequest, PlaybackState, StopReason},
    room::{RoomCloseReason, RoomHandle, RoomId, RoomManager, RoomRequest, RoomState, UserRole},
};

#[derive(Debug, Clone)]
pub enum SessionMsg {
    RoomState(RoomState),
    RoomClosed(RoomCloseReason),
    PlaybackHosting,
    PlaybackAvailable(PlaybackInfo),
    PlaybackStarted,
    PlaybackConnected,
    PlaybackSync(PlaybackState),
    PlaybackStopped(StopReason),
    PlaybackDisconnected(DisconnectReason),
}

#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub id: SessionId,
    pub name: String,
    time_offset: Weak<AtomicI64>,
    message_tx: mpsc::WeakSender<SessionMsg>,
}

impl SessionHandle {
    pub async fn send_message(&self, msg: SessionMsg) -> anyhow::Result<bool> {
        let Some(message_tx) = self.message_tx.upgrade() else {
            return Ok(false);
        };
        message_tx.send(msg).await?;
        Ok(true)
    }

    pub fn time_offset(&self) -> i64 {
        self.time_offset
            .upgrade()
            .map(|t| t.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}

pub struct Session {
    id: SessionId,
    running: bool,
    room_manager: Arc<sync::Mutex<RoomManager>>,
    room: Option<RoomHandle>,
    message_tx: mpsc::Sender<SessionMsg>,
    message_rx: mpsc::Receiver<SessionMsg>,
    connection: Connection,
    ping_interval: time::Interval,
    time_offset: Arc<AtomicI64>,
}

impl Session {
    const PING_INTERVAL: Duration = Duration::from_secs(5);

    pub fn new(connection: Connection, room_manager: Arc<sync::Mutex<RoomManager>>) -> Self {
        let (message_tx, message_rx) = mpsc::channel::<SessionMsg>(32);
        Self {
            id: SessionId::new(),
            running: true,
            room: None,
            message_rx,
            message_tx,
            connection,
            room_manager,
            time_offset: Arc::new(0.into()),
            ping_interval: time::interval(Self::PING_INTERVAL),
        }
    }

    pub async fn run(&mut self) {
        log::debug!("Starting session for user '{}'", self.connection.username());
        log::info!("User '{}' connected.", self.connection.username());
        while self.running {
            tokio::select! {
                client_msg = self.connection.recv() => {
                    if let Some(msg) = client_msg {
                        self.handle_client_msg(msg).await
                    } else {
                        // the connection was closed
                        self.running = false;
                    }
                }
                session_msg = self.message_rx.recv() => {
                    if let Some(msg) = session_msg {
                        self.handle_session_msg(msg).await
                    } else {
                        self.running = false;
                        log::error!("The session message channel was unexpectedly closed!");
                        if let Err(err) = self.connection.close(CloseReason::ServerError, "Your session crashed").await {
                            log::error!("Failed to close connection: {err:?}");
                        }
                    }
                },
                _ = self.ping_interval.tick() => self.ping().await
            }
        }
        if let Err(error) = self.leave_room().await {
            log::error!("Failed to leave room after session termination: {error:?}");
        }
    }

    async fn ping(&mut self) {
        match self.connection.ping().await {
            Ok(Some(result)) => self
                .time_offset
                .store(result.time_offset, Ordering::Relaxed),
            Ok(None) => (), // the connection was closed; this is handled separately
            Err(err) => log::debug!("Failed to ping client: {err:?}"),
        };
    }

    async fn create_room(&mut self, name: String, password: String) -> anyhow::Result<()> {
        log::debug!(
            "Session {} requested to create a room named '{name}'",
            self.id
        );
        if !self.connection.permissions().host {
            return Err(anyhow!("Your account is not permitted to host rooms"));
        }

        self.leave_room()
            .await
            .context("Failed to leave current room before opening a new one")?;

        log::info!(
            "User '{}' is creating room '{name}'",
            self.connection.username()
        );

        let room_handle = self
            .room_manager
            .lock()
            .await
            .create_room(name, password, self.get_handle())
            .await?;
        self.room = Some(room_handle);

        self.connection
            .send(Message::new(MessageBody::RoomCreateAckV1))
            .await
            .context("Failed to send ACK message")?;

        Ok(())
    }

    async fn close_room(&mut self) -> anyhow::Result<()> {
        log::debug!("Session {} requested to close its room", self.id);
        let Some(room_handle) = &self.room else {
            return Ok(());
        };

        if !room_handle.role.permissions().can_close {
            return Err(anyhow!("Not authorized to close the room"));
        }

        log::info!(
            "User '{}' is closing room '{}'",
            self.connection.username(),
            room_handle.name
        );

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

    async fn join_room(&mut self, room_id: RoomId, password: String) -> anyhow::Result<()> {
        log::debug!("Session {} requested to join room {room_id}", self.id);
        self.leave_room()
            .await
            .context("Failed to leave current room before joining a new one")?;

        let mut room_mgr = self.room_manager.lock().await;

        if Some(password) != room_mgr.get_room_password(room_id) {
            return Err(anyhow!("Incorrect password"));
        }

        let room_handle = room_mgr.join_room(room_id, self.get_handle()).await?;

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
        if self.room.is_none() {
            return Ok(());
        }

        log::debug!("Session {} requested to leave its room", self.id);
        self.send_room_msg(RoomRequest::Leave(self.id)).await?;
        self.room = None;
        let result = self
            .connection
            .send(Message::new(MessageBody::RoomLeaveAckV1))
            .await;
        if let Err(err) = result {
            log::debug!("Failed to send room leave ACK; assuming the connection is closed: {err:?}")
        }
        Ok(())
    }

    async fn kick(&mut self, session_id: SessionId) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Ok(());
        };

        if !room.role.permissions().can_kick {
            return Err(anyhow!("Not authorized to kick users"));
        }

        log::debug!("Session {} requested to kick {}", self.id, session_id);
        self.send_room_msg(RoomRequest::Leave(session_id)).await?;
        Ok(())
    }

    async fn set_user_role(&mut self, session_id: SessionId, role: UserRole) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Ok(());
        };

        if !room.role.permissions().can_set_roles {
            return Err(anyhow!("Not authorized to set user roles"));
        }

        log::debug!(
            "Session {} requested to set role for {} to {:?}",
            self.id,
            session_id,
            role
        );
        self.send_room_msg(RoomRequest::SetRole(session_id, role))
            .await?;
        Ok(())
    }

    async fn send_room_permissions(&mut self) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Err(anyhow!("Not currently in a room"));
        };

        log::debug!(
            "Session {} requested its permissions; the user role is {}",
            self.id,
            room.role
        );
        self.connection
            .send(Message::new(MessageBody::RoomPermissionsV1(
                dto::RoomPermissionsMsgBodyV1 {
                    role: room.role.into(),
                    permissions: room.role.permissions().into(),
                },
            )))
            .await
    }

    async fn host_playback(&mut self) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Err(anyhow!("Not currently in a room"));
        };

        if !room.role.permissions().can_host {
            return Err(anyhow!("Not authorized to host playback"));
        }

        log::debug!("Session {} requested to host playback", self.id);
        self.send_room_msg(RoomRequest::PlaybackHost(self.id))
            .await?;

        Ok(())
    }

    async fn connect_playback(&mut self) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Err(anyhow!("Not currently in a room"));
        };

        if !room.role.permissions().can_host {
            return Err(anyhow!("Not authorized to host playback"));
        }

        log::debug!("Session {} requested to connect to playback", self.id);
        self.send_room_msg(RoomRequest::PlaybackConnect(self.id))
            .await?;

        Ok(())
    }

    async fn playback_request(&mut self, request: PlaybackRequest) -> anyhow::Result<()> {
        self.send_room_msg(RoomRequest::Playback(self.id, request))
            .await?;

        Ok(())
    }

    async fn send_room_msg(&mut self, msg: RoomRequest) -> anyhow::Result<()> {
        let Some(room_handle) = &mut self.room else {
            return Err(anyhow!("Not currently in a room"));
        };
        if !room_handle.send_request(msg).await? {
            log::warn!("Room {} was unexpectedly closed", room_handle.id);
            self.room = None;
            self.connection
                .send(Message::new(MessageBody::RoomDisconnectedV1(
                    dto::RoomDisconnectedMsgBodyV1 {
                        reason: dto::RoomDisconnectedReasonV1::ServerError,
                    },
                )))
                .await
                .context("Failed to send disconnect message")?;
            return Ok(());
        };
        Ok(())
    }

    async fn request_state(&mut self) -> anyhow::Result<()> {
        self.send_room_msg(RoomRequest::GetState).await
    }

    async fn handle_client_msg(&mut self, msg: Message) {
        let result = match msg.body {
            MessageBody::RoomCreateV1(body) => self.create_room(body.name, body.password).await,
            MessageBody::RoomCloseV1 => self.close_room().await,
            MessageBody::RoomJoinV1(body) => self.join_room(body.id.into(), body.password).await,
            MessageBody::RoomLeaveV1 => self.leave_room().await,
            MessageBody::RoomRequestStateV1 => self.request_state().await,
            MessageBody::RoomRequestPermissionsV1 => self.send_room_permissions().await,
            MessageBody::RoomSetUserRole(body) => {
                self.set_user_role(body.user_id.into(), body.role.into())
                    .await
            }
            MessageBody::RoomKickUser(body) => self.kick(body.user_id.into()).await,
            MessageBody::PlaybackRequestHostV1 => self.host_playback().await,
            MessageBody::PlaybackRequestConnectV1 => self.connect_playback().await,
            MessageBody::PlaybackRequestStartV1(body) => {
                self.playback_request(PlaybackRequest::Start(body.source.into()))
                    .await
            }
            MessageBody::PlaybackSyncV1(body) => {
                self.playback_request(PlaybackRequest::Sync(body.state.into()))
                    .await
            }
            MessageBody::PlaybackRequestStopV1 => {
                self.playback_request(PlaybackRequest::Stop(StopReason::StoppedByHost))
                    .await
            }
            MessageBody::PlaybackRequestDisconnectV1 => {
                self.playback_request(PlaybackRequest::Disconnect(DisconnectReason::User))
                    .await
            }
            _ => Ok(()),
        };
        if let Some(err) = result.err() {
            log::error!("Failed to handle message: {err:?}");
            self.connection.send_error(err).await;
        }
    }

    async fn send_room_state(&mut self, state: RoomState) -> anyhow::Result<()> {
        self.send_message(MessageBody::RoomStateV1(dto::RoomStateMsgBodyV1::from(
            state,
        )))
        .await
    }

    async fn room_closed(&mut self, reason: RoomCloseReason) -> anyhow::Result<()> {
        self.room = None;
        self.send_message(MessageBody::RoomDisconnectedV1(
            dto::RoomDisconnectedMsgBodyV1 {
                reason: match reason {
                    RoomCloseReason::ServerError => dto::RoomDisconnectedReasonV1::ServerError,
                    RoomCloseReason::ClosedByHost => dto::RoomDisconnectedReasonV1::ClosedByHost,
                },
            },
        ))
        .await
    }

    async fn send_message(&mut self, body: MessageBody) -> anyhow::Result<()> {
        self.connection.send(Message::new(body)).await
    }

    async fn handle_session_msg(&mut self, msg: SessionMsg) {
        let result = match msg {
            SessionMsg::RoomState(state) => self.send_room_state(state).await,
            SessionMsg::RoomClosed(reason) => self.room_closed(reason).await,
            SessionMsg::PlaybackHosting => self.send_message(MessageBody::PlaybackHosting).await,
            SessionMsg::PlaybackAvailable(info) => {
                self.send_message(MessageBody::PlaybackAvailableV1(
                    dto::PlaybackAvailableMsgBodyV1 { info: info.into() },
                ))
                .await
            }
            SessionMsg::PlaybackStarted => self.send_message(MessageBody::PlaybackStartedV1).await,
            SessionMsg::PlaybackConnected => {
                self.send_message(MessageBody::PlaybackConnectedV1).await
            }
            SessionMsg::PlaybackSync(state) => {
                self.send_message(MessageBody::PlaybackSyncV1(dto::PlaybackSyncMsgBodyV1 {
                    state: state.into(),
                }))
                .await
            }
            SessionMsg::PlaybackStopped(reason) => {
                self.send_message(MessageBody::PlaybackStoppedV1(
                    dto::PlaybackStoppedMsgBodyV1 {
                        reason: reason.into(),
                    },
                ))
                .await
            }
            SessionMsg::PlaybackDisconnected(reason) => {
                self.send_message(MessageBody::PlaybackDisconnectedV1(
                    dto::PlaybackDisconnectedMsgBodyV1 {
                        reason: reason.into(),
                    },
                ))
                .await
            }
        };
        if let Some(err) = result.err() {
            log::error!("Failed to handle session message: {err:?}");
        }
    }

    fn get_handle(&self) -> SessionHandle {
        SessionHandle {
            id: self.id,
            name: self.connection.username().to_string(),
            time_offset: Arc::downgrade(&self.time_offset),
            message_tx: self.message_tx.clone().downgrade(),
        }
    }
}
