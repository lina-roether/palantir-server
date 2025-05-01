use std::process::ExitCode;

mod api_access;
mod app;
mod config;
mod connection;
mod messages;
mod playback;
mod room;
mod session;
mod utils;

#[tokio::main]
async fn main() -> ExitCode {
    let result = app::start().await;
    match result {
        Ok(..) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:?}");
            ExitCode::FAILURE
        }
    }
}
