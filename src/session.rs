use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use log::{debug, error};
use parking_lot::{Mutex, RwLock};
use tokio::{
    sync::mpsc,
    task::{AbortHandle, JoinHandle},
};
use uuid::Uuid;

use crate::{
    connection::Connection,
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

struct SessionState {
    time_offset: i64,
}

#[allow(clippy::derivable_impls)]
impl Default for SessionState {
    fn default() -> Self {
        Self { time_offset: 0 }
    }
}

struct Session {
    user: User,
    state: Arc<Mutex<SessionState>>,
    command_tx: mpsc::Sender<SessionCommand>,
    message_rx: mpsc::Receiver<Message>,
    session_handle: JoinHandle<()>,
    timer_handle: AbortHandle,
}

struct SessionThread {
    open: bool,
    state: Arc<Mutex<SessionState>>,
    message_tx: mpsc::Sender<Message>,
    connection: Connection,
}

impl SessionThread {
    fn new(
        state: Arc<Mutex<SessionState>>,
        connection: Connection,
        message_tx: mpsc::Sender<Message>,
    ) -> Self {
        Self {
            open: true,
            state,
            message_tx,
            connection,
        }
    }

    async fn send(&mut self, message: Message) -> anyhow::Result<()> {
        self.connection.send(message).await
    }

    async fn recv(&mut self) -> Option<Message> {
        self.connection.recv().await
    }

    async fn close(&mut self, reason: SessionCloseReason, message: String) -> anyhow::Result<()> {
        self.connection
            .send(Message::new(MessageBody::SessionTerminatedV1(
                SessionTerminatedMsgBodyV1 {
                    reason: match reason {
                        SessionCloseReason::ClosedByHost => SessionTerminateReasonV1::ClosedByHost,
                        SessionCloseReason::Unauthorized => SessionTerminateReasonV1::Unauthorized,
                        SessionCloseReason::ServerError => SessionTerminateReasonV1::ServerError,
                    },
                    message,
                },
            )))
            .await?;
        self.open = false;
        Ok(())
    }

    async fn leave(&mut self) -> anyhow::Result<()> {
        self.open = false;
        self.connection
            .send(Message::new(MessageBody::SessionLeaveAckV1))
            .await?;
        Ok(())
    }

    async fn close_server_error(
        &mut self,
        err: impl fmt::Display + fmt::Debug,
    ) -> anyhow::Result<()> {
        let message = if cfg!(debug_assertions) {
            err.to_string()
        } else {
            "Internal Server Error".to_string()
        };
        error!("Session closed: {err:?}");
        self.close(SessionCloseReason::ServerError, message).await
    }

    async fn update_time_offset(&mut self) -> anyhow::Result<()> {
        let Some(result) = self.connection.ping().await? else {
            return Err(anyhow!("Connection was closed!"));
        };
        self.state.lock().time_offset = result.time_offset;
        Ok(())
    }

    async fn handle_cmd(&mut self, cmd: Option<SessionCommand>) -> anyhow::Result<()> {
        let Some(cmd) = cmd else {
            self.close_server_error(anyhow!("Command channel closed for session thread"))
                .await
                .context("Failed to close session after server error")?;
            return Ok(());
        };
        match cmd {
            SessionCommand::Send(msg) => self.send(msg).await,
            SessionCommand::UpdateTimeOffset => self.update_time_offset().await,
            SessionCommand::Close { reason, message } => self.close(reason, message).await,
        }
    }

    async fn handle_msg_generic(&mut self, msg: Option<Message>) -> anyhow::Result<()> {
        let Some(msg) = msg else {
            debug!("Connection was closed");
            return Ok(());
        };
        match msg.body {
            MessageBody::SessionLeaveV1 => self.leave().await.context("Failed to leave session")?,
            MessageBody::SessionStateV1(..) | MessageBody::SessionTerminatedV1(..) => {
                self.connection.send_error("Unexpected message type").await;
            }
            MessageBody::SessionStartV1(..) | MessageBody::SessionJoinV1(..) => {
                self.connection.send_error("Already in a session!").await;
            }
            _ => self.message_tx.send(msg).await?,
        }
        Ok(())
    }

    async fn run(&mut self, mut command_rx: mpsc::Receiver<SessionCommand>) {
        while self.open {
            let result = tokio::select! {
                cmd = command_rx.recv() => self.handle_cmd(cmd).await,
                _msg = self.recv() => todo!()
            };
            if let Err(err) = result {
                error!("Error occurred on session thread: {err:?}");
            }
        }
    }
}

impl Session {
    const PING_INTERVAL: Duration = Duration::from_secs(30);

    async fn new(user: User, mut connection: Connection) -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(SessionState::default()));
        let (command_tx, command_rx) = mpsc::channel::<SessionCommand>(16);
        let (message_tx, message_rx) = mpsc::channel::<Message>(64);
        let session_handle =
            Self::start_session_thread(&user, &state, connection, command_rx, message_tx).await?;
        let timer_handle = Self::start_ping_timer(command_tx.clone());
        Ok(Self {
            user,
            state: Arc::new(Mutex::new(SessionState::default())),
            command_tx,
            message_rx,
            session_handle,
            timer_handle,
        })
    }

    async fn start_session_thread(
        user: &User,
        state: &Arc<Mutex<SessionState>>,
        mut connection: Connection,
        command_rx: mpsc::Receiver<SessionCommand>,
        message_tx: mpsc::Sender<Message>,
    ) -> anyhow::Result<JoinHandle<()>> {
        match user.role {
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
                Ok(tokio::spawn(Self::host_session(
                    Arc::clone(state),
                    command_rx,
                    message_tx,
                    connection,
                )))
            }
            UserRole::Guest => Ok(tokio::spawn(Self::guest_session(
                Arc::clone(state),
                command_rx,
                message_tx,
                connection,
            ))),
        }
    }

    fn start_ping_timer(command_tx: mpsc::Sender<SessionCommand>) -> AbortHandle {
        tokio::spawn(async move {
            loop {
                if let Err(err) = command_tx.send(SessionCommand::UpdateTimeOffset).await {
                    error!("Error occurred on ping timer thread: {err:?}");
                    break;
                }
                tokio::time::sleep(Self::PING_INTERVAL).await;
            }
        })
        .abort_handle()
    }

    async fn close(self, reason: SessionCloseReason, message: String) -> anyhow::Result<()> {
        self.timer_handle.abort();
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
        state: Arc<Mutex<SessionState>>,
        command_rx: mpsc::Receiver<SessionCommand>,
        message_tx: mpsc::Sender<Message>,
        connection: Connection,
    ) {
        let mut thread = SessionThread::new(state, connection, message_tx);
        thread.run(command_rx).await;
    }

    async fn guest_session(
        state: Arc<Mutex<SessionState>>,
        command_rx: mpsc::Receiver<SessionCommand>,
        message_tx: mpsc::Sender<Message>,
        connection: Connection,
    ) {
        let mut thread = SessionThread::new(state, connection, message_tx);
        thread.run(command_rx).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCloseReason {
    ClosedByHost,
    Unauthorized,
    ServerError,
}

#[derive(Debug, Clone)]
enum SessionCommand {
    Send(Message),
    UpdateTimeOffset,
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
