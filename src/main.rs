use std::time::{SystemTime, UNIX_EPOCH};

use messages::{Message, MessageBody, SessionStateMsgBody, SessionUser, SessionUserRole};
use uuid::Uuid;

mod media;
mod messages;
mod playback;
mod session;
mod user;

fn main() {
    let message = Message {
        version: 1,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before unix epoch!")
            .as_millis()
            .try_into()
            .expect("Timestamp overflow!"),
        body: MessageBody::SessionState(SessionStateMsgBody {
            id: Uuid::new_v4(),
            password: "cooles passwort".to_string(),
            users: vec![
                SessionUser {
                    name: "niko".to_string(),
                    role: SessionUserRole::Host,
                },
                SessionUser {
                    name: "bcnyyy".to_string(),
                    role: SessionUserRole::Guest,
                },
            ],
        }),
    };

    println!(
        "{}",
        // serde_json::to_string(&message).unwrap()
        String::from_utf8_lossy(&rmp_serde::to_vec(&message).unwrap())
    )
}
