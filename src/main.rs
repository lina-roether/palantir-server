use config::read_config;
use futures_util::SinkExt;
use log::error;
use messages::{Message, MessageBody};
use server::Server;

mod api_access;
mod config;
mod messages;
mod server;
mod session;
mod user;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let config = read_config(None);

    let server = match Server::bind(config).await {
        Ok(server) => server,
        Err(err) => {
            error!("Failed to start server: {err:?}");
            return;
        }
    };

    server
        .listen(|_stream, mut sink| async move {
            sink.send(Message {
                version: 1,
                timestamp: 69,
                body: MessageBody::TimingPing,
            })
            .await?;
            Ok(())
        })
        .await;
}
