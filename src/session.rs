use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use log::{debug, error, warn};
use tokio::{
    sync::{self, mpsc},
    time,
};
use uuid::Uuid;

use crate::{
    connection::{CloseReason, Connection},
    messages::{
        Message, MessageBody, RoomDisconnectedMsgBodyV1, RoomDisconnectedReasonV1,
        RoomPermissionsMsgBodyV1, RoomStateMsgBodyV1, RoomUserV1,
    },
    room::{self, RoomCloseReason, RoomHandle, RoomManager, RoomMsg, RoomState, UserRole},
};

#[derive(Debug, Clone)]
pub enum SessionMsg {
    RoomState(RoomState),
    RoomClosed(RoomCloseReason),
}

pub struct Session {
    id: Uuid,
    running: bool,
    room_manager: Arc<sync::Mutex<RoomManager>>,
    room: Option<RoomHandle>,
    message_tx: mpsc::Sender<SessionMsg>,
    message_rx: mpsc::Receiver<SessionMsg>,
    connection: Connection,
    ping_interval: time::Interval,
    time_offset: i64,
}

impl Session {
    const PING_INTERVAL: Duration = Duration::from_secs(5);

    pub fn new(connection: Connection, room_manager: Arc<sync::Mutex<RoomManager>>) -> Self {
        let (message_tx, message_rx) = mpsc::channel::<SessionMsg>(32);
        Self {
            id: Uuid::new_v4(),
            running: true,
            room: None,
            message_rx,
            message_tx,
            connection,
            room_manager,
            time_offset: 0,
            ping_interval: time::interval(Self::PING_INTERVAL),
        }
    }

    pub async fn run(&mut self) {
        debug!("Starting session for user '{}'", self.connection.username());
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
                        error!("The session message channel was unexpectedly closed!");
                        if let Err(err) = self.connection.close(CloseReason::ServerError, "Your session crashed").await {
                            error!("Failed to close connection: {err:?}");
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
            Ok(Some(result)) => self.time_offset = result.time_offset,
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
        log::debug!("Session {} requested to close its room", self.id);
        let Some(room_handle) = &self.room else {
            return Ok(());
        };

        if !room_handle.role.permissions().can_close {
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
        log::debug!("Session {} requested to join room {room_id}", self.id);
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
        if self.room.is_none() {
            return Ok(());
        }

        log::debug!("Session {} requested to leave its room", self.id);
        self.send_room_msg(RoomMsg::Leave(self.id)).await?;
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

    async fn kick(&mut self, session_id: Uuid) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Ok(());
        };

        if !room.role.permissions().can_kick {
            return Err(anyhow!("Not authorized to kick users"));
        }

        log::debug!("Session {} requested to kick {}", self.id, session_id);
        self.send_room_msg(RoomMsg::Leave(session_id)).await?;
        Ok(())
    }

    async fn set_user_role(&mut self, session_id: Uuid, role: UserRole) -> anyhow::Result<()> {
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
        self.send_room_msg(RoomMsg::SetRole(session_id, role))
            .await?;
        Ok(())
    }

    async fn send_room_permissions(&mut self) -> anyhow::Result<()> {
        let Some(room) = &self.room else {
            return Err(anyhow!("Not currently in a room"));
        };

        log::debug!(
            "Session {} requested its permissions; the user role is {:?}",
            self.id,
            room.role
        );
        self.connection
            .send(Message::new(MessageBody::RoomPermissionsV1(
                RoomPermissionsMsgBodyV1 {
                    role: room.role.into(),
                    permissions: room.role.permissions().into(),
                },
            )))
            .await
    }

    async fn send_room_msg(&mut self, msg: RoomMsg) -> anyhow::Result<()> {
        let Some(room_handle) = &self.room else {
            return Err(anyhow!("Not currently in a room"));
        };
        let Some(message_tx) = room_handle.message_tx.upgrade() else {
            warn!("Room {} was unexpectedly closed", room_handle.id);
            self.room = None;
            self.connection
                .send(Message::new(MessageBody::RoomDisconnectedV1(
                    RoomDisconnectedMsgBodyV1 {
                        reason: RoomDisconnectedReasonV1::ServerError,
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
            MessageBody::RoomRequestPermissionsV1 => self.send_room_permissions().await,
            MessageBody::RoomSetUserRole(body) => {
                self.set_user_role(body.user_id, body.role.into()).await
            }
            MessageBody::RoomKickUser(body) => self.kick(body.user_id).await,
            _ => Ok(()),
        };
        if let Some(err) = result.err() {
            error!("Failed to handle message: {err:?}");
            self.connection.send_error(err).await;
        }
    }

    async fn send_room_state(&mut self, state: RoomState) -> anyhow::Result<()> {
        self.connection
            .send(Message::new(MessageBody::RoomStateV1(RoomStateMsgBodyV1 {
                id: state.id,
                name: state.name,
                password: state.password,
                users: state
                    .users
                    .iter()
                    .map(|user| RoomUserV1 {
                        id: user.id,
                        name: user.name.clone(),
                        role: user.role.into(),
                    })
                    .collect(),
            })))
            .await
    }

    async fn room_closed(&mut self, reason: RoomCloseReason) -> anyhow::Result<()> {
        self.room = None;
        self.connection
            .send(Message::new(MessageBody::RoomDisconnectedV1(
                RoomDisconnectedMsgBodyV1 {
                    reason: match reason {
                        RoomCloseReason::ServerError => RoomDisconnectedReasonV1::ServerError,
                        RoomCloseReason::ClosedByHost => RoomDisconnectedReasonV1::ClosedByHost,
                    },
                },
            )))
            .await
    }

    async fn handle_session_msg(&mut self, msg: SessionMsg) {
        let result = match msg {
            SessionMsg::RoomState(state) => self.send_room_state(state).await,
            SessionMsg::RoomClosed(reason) => self.room_closed(reason).await,
        };
        if let Some(err) = result.err() {
            error!("Failed to handle session message: {err:?}");
        }
    }

    fn session_info(&self) -> room::SessionInfo {
        room::SessionInfo {
            session_id: self.id,
            session_message_tx: self.message_tx.clone().downgrade(),
            name: self.connection.username().to_string(),
        }
    }
}
