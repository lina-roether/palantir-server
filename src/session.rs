use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use tokio::time::timeout;
use uuid::Uuid;

use crate::{
    listener::{MessageSink, MessageStream},
    messages::{Message, MessageBody},
    utils::timestamp,
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
    name: String,
    role: UserRole,
    time_offset: i32,
    message_stream: MessageStream,
    message_sink: MessageSink,
}

struct Session {
    password: String,
    users: HashMap<Uuid, User>,
    media: Option<Media>,
}

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

impl Session {
    fn new(password: String) -> Self {
        Self {
            password,
            users: HashMap::new(),
            media: None,
        }
    }

    async fn add_user(
        &mut self,
        name: String,
        role: UserRole,
        message_stream: MessageStream,
        message_sink: MessageSink,
    ) -> Result<()> {
        let mut user = User {
            name,
            role,
            time_offset: 0,
            message_stream,
            message_sink,
        };
        todo!()
    }

    async fn get_time_offset(
        message_stream: &mut MessageStream,
        message_sink: &mut MessageSink,
    ) -> Result<i32> {
        message_sink
            .send(Message {
                timestamp: timestamp(),
                body: MessageBody::ConnectionPingV1,
            })
            .await?;

        todo!()
    }
}

struct SessionManager {
    sessions: HashMap<Uuid, RwLock<Session>>,
}

impl SessionManager {
    fn start_session(&mut self, password: String) -> Uuid {
        let id = Uuid::new_v4();
        self.sessions
            .insert(id, RwLock::new(Session::new(password)));
        id
    }

    fn get_session(&self, id: Uuid) -> Option<&RwLock<Session>> {
        self.sessions.get(&id)
    }

    fn stop_session(&mut self, id: Uuid) {
        self.sessions.remove(&id);
    }
}
