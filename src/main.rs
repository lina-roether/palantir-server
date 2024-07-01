use messages::{MediaSelectMsgBody, Message, MessageBody};

mod media;
mod messages;
mod playback;
mod session;
mod user;

fn main() {
    let message = Message {
        version: 1,
        body: MessageBody::MediaSelect(MediaSelectMsgBody {
            page_href: "https://example.com".to_string(),
            frame_href: "https://video.example.com".to_string(),
            element_query: "#player".to_string(),
        }),
    };

    println!("{}", serde_json::to_string(&message).unwrap())
}
