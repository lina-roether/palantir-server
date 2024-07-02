use config::read_config;
use futures_util::SinkExt;
use listener::Listener;
use log::error;
use messages::{Message, MessageBody};

mod api_access;
mod config;
mod listener;
mod messages;
mod session;
mod user;
mod utils;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let config = read_config(None);

    let server = match Listener::bind(config).await {
        Ok(server) => server,
        Err(err) => {
            error!("Failed to start server: {err:?}");
            return;
        }
    };

    server
        .listen(|_stream, mut sink| async move {
            sink.send(Message {
                timestamp: 69,
                body: MessageBody::ConnectionPingV1,
            })
            .await?;
            Ok(())
        })
        .await;
}
