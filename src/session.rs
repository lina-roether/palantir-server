use std::{collections::HashMap, sync::RwLock};

use uuid::Uuid;

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
    key: Box<[u8]>,
    role: UserRole,
    time_offset: i32,
}

struct Session {
    password: String,
    users: HashMap<Uuid, User>,
    media: Option<Media>,
}

impl Session {
    fn new(password: String) -> Self {
        Self {
            password,
            users: HashMap::new(),
            media: None,
        }
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

    fn stop_session(&mut self, id: Uuid) {
        self.sessions.remove(&id);
    }
}
