use std::collections::HashMap;

use uuid::Uuid;

use crate::{media::Media, user::User};

struct Session {
    password: String,
    users: Vec<User>,
    media: Option<Media>,
}

impl Session {
    fn new(password: String) -> Self {
        Self {
            password,
            users: Vec::new(),
            media: None,
        }
    }
}

struct SessionManager {
    sessions: HashMap<Uuid, Session>,
}

impl SessionManager {
    fn start_session(&mut self, password: String) -> Uuid {
        let id = Uuid::new_v4();
        self.sessions.insert(id, Session::new(password));
        id
    }

    fn stop_session(&mut self, id: Uuid) {
        self.sessions.remove(&id);
    }
}
