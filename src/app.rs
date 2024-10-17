use std::sync::Arc;

use clap::Parser;
use log::LevelFilter;
use tokio::sync;

use crate::{
    api_access::ApiAccessManager, config::Config, connection::ConnectionListener,
    room::RoomManager, session::Session,
};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    #[arg(
        short,
        long,
        help = "The port or URL that the server should listen on. This overrides the value from the config file."
    )]
    pub listen_on: Option<String>,

    #[arg(
        short,
        long,
        help = "The path to the config file. The default is `config.toml`."
    )]
    pub config: Option<String>,
}

pub async fn start() -> anyhow::Result<()> {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Info)
        .parse_env("PALANTIR_LOG")
        .init();

    let cli = Cli::parse();
    let config = Config::from_cli_args(&cli)?;

    let access_mgr = Arc::new(ApiAccessManager::new(config.api_access));
    let room_mgr = Arc::new(sync::Mutex::new(RoomManager::new()));

    let listener = ConnectionListener::bind(config.server).await?;
    listener
        .listen(move |mut conn| {
            let access_mgr = Arc::clone(&access_mgr);
            let room_mgr = Arc::clone(&room_mgr);
            async move {
                conn.init(&access_mgr).await?;

                let mut session = Session::new(conn, room_mgr);
                session.run().await;

                Ok(())
            }
        })
        .await?;

    Ok(())
}
