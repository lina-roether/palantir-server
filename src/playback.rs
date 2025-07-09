use std::collections::HashMap;

use anyhow::{anyhow, Context};

use crate::{
    messages::dto,
    session::{SessionHandle, SessionId, SessionMsg},
};

#[derive(Debug, Clone)]
pub struct PlaybackInfo {
    pub host: String,
    pub source: Option<PlaybackSource>,
}

impl From<PlaybackInfo> for dto::RoomPlaybackInfoV1 {
    fn from(value: PlaybackInfo) -> Self {
        Self {
            host: value.host.into(),
            source: value.source.map(Into::into),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlaybackSource {
    pub title: String,
    pub page_href: String,
    pub frame_href: String,
    pub element_query: String,
}

impl From<PlaybackSource> for dto::PlaybackSourceV1 {
    fn from(value: PlaybackSource) -> Self {
        Self {
            title: value.title,
            page_href: value.page_href,
            frame_href: value.frame_href,
            element_query: value.element_query,
        }
    }
}

impl From<dto::PlaybackSourceV1> for PlaybackSource {
    fn from(value: dto::PlaybackSourceV1) -> Self {
        Self {
            title: value.title,
            page_href: value.page_href,
            frame_href: value.frame_href,
            element_query: value.element_query,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlaybackState {
    pub timestamp: u64,
    pub playing: bool,
    pub time: f32,
}

impl PlaybackState {
    fn normalize_offset(&self, source_offset: i64) -> Self {
        Self {
            timestamp: self.timestamp.saturating_add_signed(-source_offset),
            ..self.clone()
        }
    }

    fn incorporate_offset(&self, dest_offset: i64) -> Self {
        Self {
            timestamp: self.timestamp.saturating_add_signed(dest_offset),
            ..self.clone()
        }
    }
}

impl From<dto::PlaybackStateV1> for PlaybackState {
    fn from(value: dto::PlaybackStateV1) -> Self {
        Self {
            timestamp: value.timestamp,
            playing: value.playing,
            time: value.time,
        }
    }
}

impl From<PlaybackState> for dto::PlaybackStateV1 {
    fn from(value: PlaybackState) -> Self {
        Self {
            timestamp: value.timestamp,
            playing: value.playing,
            time: value.time,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum StopReason {
    HostError,
    StoppedByHost,
    Superseded,
}

impl From<StopReason> for dto::PlaybackStopReasonV1 {
    fn from(value: StopReason) -> Self {
        match value {
            StopReason::HostError => Self::HostError,
            StopReason::StoppedByHost => Self::StoppedByHost,
            StopReason::Superseded => Self::Superseded,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DisconnectReason {
    User,
    Stopped(StopReason),
    SubscriberError,
}

impl From<DisconnectReason> for dto::PlaybackDisconnectReasonV1 {
    fn from(value: DisconnectReason) -> Self {
        match value {
            DisconnectReason::User => Self::User,
            DisconnectReason::Stopped(reason) => Self::Stopped(reason.into()),
            DisconnectReason::SubscriberError => Self::SubscriberError,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlaybackRequest {
    Start(PlaybackSource),
    Disconnect(DisconnectReason),
    Stop(StopReason),
    Sync(PlaybackState),
}

#[derive(Debug, Clone)]
pub struct Playback {
    running: bool,
    source: Option<PlaybackSource>,
    host: SessionHandle,
    subscribers: HashMap<SessionId, SessionHandle>,
}

impl Playback {
    pub fn new(host: SessionHandle) -> Self {
        Self {
            running: false,
            source: None,
            host,
            subscribers: HashMap::new(),
        }
    }

    pub fn get_info(&self) -> PlaybackInfo {
        PlaybackInfo {
            source: self.source.clone(),
            host: self.host.name.clone(),
        }
    }

    pub async fn handle_request(
        &mut self,
        session_id: SessionId,
        request: PlaybackRequest,
    ) -> anyhow::Result<()> {
        let is_host = session_id == self.host.id;
        if !is_host && !self.subscribers.contains_key(&session_id) {
            return Err(anyhow!("Users who are neither the playback host nor a subscriber cannot send playback requests"));
        };

        match request {
            PlaybackRequest::Start(source) => {
                if !is_host {
                    return Err(anyhow!("Only the playback host can start playback"));
                }
                self.start(source).await?;
            }
            PlaybackRequest::Disconnect(reason) => self.disconnect(session_id, reason).await?,
            PlaybackRequest::Stop(reason) => {
                if !is_host {
                    return Err(anyhow!("Only the playback host can stop playback"));
                }
                self.stop(reason).await?;
            }
            PlaybackRequest::Sync(state) => self.sync(session_id, state).await?,
        }

        Ok(())
    }

    async fn start(&mut self, source: PlaybackSource) -> anyhow::Result<()> {
        if self.running {
            return Ok(());
        }
        self.running = true;
        self.source = Some(source);
        if !self.host.send_message(SessionMsg::PlaybackStarted).await? {
            self.stop(StopReason::HostError)
                .await
                .context("Failed to stop playback after host error")?;
        }

        for (id, subscriber) in &self.subscribers {
            if let Err(err) = subscriber
                .send_message(SessionMsg::PlaybackAvailable(self.get_info()))
                .await
            {
                log::error!("Failed to announce playback to user {id}: {err:?}");
            }
        }
        Ok(())
    }

    pub async fn stop(&mut self, reason: StopReason) -> anyhow::Result<()> {
        if !self.running {
            return Ok(());
        }
        self.source = None;
        for subscriber in self.subscribers.values() {
            subscriber
                .send_message(SessionMsg::PlaybackDisconnected(DisconnectReason::Stopped(
                    reason,
                )))
                .await?;
        }
        self.subscribers.clear();
        self.host
            .send_message(SessionMsg::PlaybackStopped(reason))
            .await?;
        Ok(())
    }

    pub async fn connect(&mut self, user: SessionHandle) -> anyhow::Result<()> {
        if !self.running {
            return Err(anyhow!(
                "Tried to connect to a playback that isn't running!"
            ));
        }
        if user.id == self.host.id {
            return Err(anyhow!(
                "The playback host can't connect to their own playback"
            ));
        }
        user.send_message(SessionMsg::PlaybackConnected).await?;
        self.subscribers.insert(user.id, user);
        Ok(())
    }

    async fn disconnect(&mut self, id: SessionId, reason: DisconnectReason) -> anyhow::Result<()> {
        if let Some(handle) = self.subscribers.remove(&id) {
            handle
                .send_message(SessionMsg::PlaybackDisconnected(reason))
                .await?;
        }
        Ok(())
    }

    async fn sync(&mut self, id: SessionId, state: PlaybackState) -> anyhow::Result<()> {
        let mut normalized_state = state.clone();
        if id == self.host.id {
            normalized_state = state.normalize_offset(self.host.time_offset());
        } else if let Some(source) = self.subscribers.get(&id) {
            normalized_state = state.normalize_offset(source.time_offset());
        }

        if id != self.host.id && !send_sync_msg(&self.host, &normalized_state).await? {
            self.stop(StopReason::StoppedByHost).await?;
            return Ok(());
        }
        let mut errored_subscribers: Vec<SessionId> = vec![];
        for target in self.subscribers.values() {
            if target.id == id {
                continue;
            }
            if !send_sync_msg(&target, &normalized_state).await? {
                errored_subscribers.push(target.id);
            }
        }

        for id in errored_subscribers {
            self.disconnect(id, DisconnectReason::SubscriberError)
                .await?;
        }

        Ok(())
    }
}

async fn send_sync_msg(session: &SessionHandle, state: &PlaybackState) -> anyhow::Result<bool> {
    session
        .send_message(SessionMsg::PlaybackSync(
            state.incorporate_offset(session.time_offset()),
        ))
        .await
}
