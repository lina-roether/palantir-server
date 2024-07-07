use std::{collections::HashMap, mem, sync::Arc};

use anyhow::{anyhow, Context};
use log::error;
use parking_lot::{Mutex, RwLock};
use tokio::{
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;

use crate::{
    connection::{CloseReason, Connection},
    messages::{Message, MessageBody, SessionTerminateReasonV1, SessionTerminatedMsgBodyV1},
};

pub struct PlaybackState {
    playing: bool,
    time: u64,
    last_update: u64,
}

pub struct Media {
    page_href: String,
    frame_href: String,
    element_query: String,
    state: PlaybackState,
}

pub enum UserRole {
    Host,
    Guest,
}

pub struct User {
    pub name: String,
    pub role: UserRole,
}

struct Session {
    user: User,
    command_tx: mpsc::Sender<SessionCommand>,
    session_handle: JoinHandle<()>,
}

impl Session {
    async fn new(user: User, mut connection: Connection) -> anyhow::Result<Self> {
        let (command_tx, command_rx) = mpsc::channel::<SessionCommand>(16);
        let session_handle = match user.role {
            UserRole::Host => {
                if !connection.permissions().host {
                    let err = anyhow!("Not authorized to host sessions!");
                    if let Err(err) = connection
                        .send(Message::new(MessageBody::SessionTerminatedV1(
                            SessionTerminatedMsgBodyV1 {
                                reason: SessionTerminateReasonV1::Unauthorized,
                                message: err.to_string(),
                            },
                        )))
                        .await
                    {
                        error!("Failed to send session terminated message: {err:?}");
                    }
                    return Err(err);
                }
                tokio::spawn(Self::host_session(command_rx, connection))
            }
            UserRole::Guest => tokio::spawn(Self::guest_session(command_rx, connection)),
        };
        Ok(Self {
            user,
            command_tx,
            session_handle,
        })
    }

    async fn close(self, reason: SessionCloseReason, message: String) -> anyhow::Result<()> {
        self.command_tx
            .send(SessionCommand::Close { reason, message })
            .await
            .context("Failed to send close command to session")?;
        self.session_handle
            .await
            .context("Failed to join session thread after close")?;
        Ok(())
    }

    async fn host_session(
        mut command_rx: mpsc::Receiver<SessionCommand>,
        mut connection: Connection,
    ) {
        loop {
            tokio::select! {
                msg = connection.recv() => {
                    todo!()
                }
                cmd = command_rx.recv() => {
                    todo!()
                }
            }
        }
    }

    async fn guest_session(command_rx: mpsc::Receiver<SessionCommand>, connection: Connection) {
        todo!()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCloseReason {
    ClosedByHost,
    Unauthorized,
}

#[derive(Debug, Clone)]
enum SessionCommand {
    Close {
        reason: SessionCloseReason,
        message: String,
    },
}

struct Room {
    password: String,
    sessions: RwLock<HashMap<Uuid, Mutex<Session>>>,
    media: Option<Media>,
}

impl Room {
    async fn run(self: Arc<Self>) {
        todo!()
    }

    async fn add_session(&self, user: User, connection: Connection) -> anyhow::Result<Uuid> {
        let session_id = Uuid::new_v4();
        let session = Session::new(user, connection).await?;
        self.sessions
            .write()
            .insert(session_id, Mutex::new(session));
        Ok(session_id)
    }

    async fn close(
        &self,
        session_id: Uuid,
        reason: SessionCloseReason,
        message: String,
    ) -> anyhow::Result<()> {
        if let Some(session) = self.remove_session(session_id) {
            session.close(reason, message).await?;
        }
        Ok(())
    }

    async fn close_all(&mut self, reason: SessionCloseReason, message: String) {
        let session_ids = self.sessions.read().keys().copied().collect::<Vec<_>>();
        let results = futures::future::join_all(
            session_ids
                .into_iter()
                .map(|session_id| self.close(session_id, reason, message.clone())),
        )
        .await;

        for result in results {
            if let Err(err) = result {
                error!("{err:?}");
            }
        }
    }

    fn remove_session(&self, session_id: Uuid) -> Option<Session> {
        self.sessions
            .write()
            .remove(&session_id)
            .map(Mutex::into_inner)
    }
}

struct RoomRegistry {
    rooms: HashMap<Uuid, Arc<RwLock<Room>>>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self {
            rooms: HashMap::new(),
        }
    }
}
