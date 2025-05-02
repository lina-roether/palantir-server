use std::collections::HashMap;

use anyhow::{anyhow, Context};
use uuid::Uuid;

use crate::{
    messages::{
        PlaybackDisconnectReasonV1, PlaybackSourceV1, PlaybackStateV1, PlaybackStopReasonV1,
        RoomPlaybackInfoV1,
    },
    session::{SessionHandle, SessionMsg},
};

#[derive(Debug, Clone)]
pub struct PlaybackInfo {
    pub user_id: Uuid,
    pub user_name: String,
    pub title: String,
    pub href: String,
}

impl From<PlaybackInfo> for RoomPlaybackInfoV1 {
    fn from(value: PlaybackInfo) -> Self {
        Self {
            user_id: value.user_id,
            user_name: value.user_name,
            title: value.title,
            href: value.href,
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

impl From<PlaybackSource> for PlaybackSourceV1 {
    fn from(value: PlaybackSource) -> Self {
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

impl From<PlaybackStateV1> for PlaybackState {
    fn from(value: PlaybackStateV1) -> Self {
        Self {
            timestamp: value.timestamp,
            playing: value.playing,
            time: value.time,
        }
    }
}

impl From<PlaybackState> for PlaybackStateV1 {
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
}

impl From<StopReason> for PlaybackStopReasonV1 {
    fn from(value: StopReason) -> Self {
        match value {
            StopReason::HostError => Self::HostError,
            StopReason::StoppedByHost => Self::StoppedByHost,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DisconnectReason {
    Stopped,
    SubscriberError,
}

impl From<DisconnectReason> for PlaybackDisconnectReasonV1 {
    fn from(value: DisconnectReason) -> Self {
        match value {
            DisconnectReason::Stopped => Self::Stopped,
            DisconnectReason::SubscriberError => Self::SubscriberError,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Playback {
    running: bool,
    source: PlaybackSource,
    host: SessionHandle,
    subscribers: HashMap<Uuid, SessionHandle>,
}

impl Playback {
    pub fn new(host: SessionHandle, source: PlaybackSource) -> Self {
        Self {
            running: false,
            source,
            host,
            subscribers: HashMap::new(),
        }
    }

    pub fn get_info(&self) -> PlaybackInfo {
        PlaybackInfo {
            title: self.source.title.clone(),
            user_id: self.host.id,
            user_name: self.host.name.clone(),
            href: self.source.page_href.clone(),
        }
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        if self.running {
            return Ok(());
        }
        self.running = true;
        if !self.host.send_message(SessionMsg::PlaybackStarted).await? {
            self.stop(StopReason::HostError)
                .await
                .context("Failed to stop playback after host error")?;
        }
        Ok(())
    }

    pub async fn stop(&mut self, reason: StopReason) -> anyhow::Result<()> {
        if !self.running {
            return Ok(());
        }
        for subscriber in self.subscribers.values() {
            subscriber
                .send_message(SessionMsg::PlaybackDisconnected(DisconnectReason::Stopped))
                .await?;
        }
        self.subscribers.clear();
        self.host
            .send_message(SessionMsg::PlaybackStopped(reason))
            .await?;
        Ok(())
    }

    pub async fn connect(&mut self, handle: SessionHandle) -> anyhow::Result<()> {
        if !self.running {
            return Err(anyhow!(
                "Tried to connect to a playback that isn't running!"
            ));
        }
        handle.send_message(SessionMsg::PlaybackConnected).await?;
        self.subscribers.insert(handle.id, handle);
        Ok(())
    }

    pub async fn disconnect(&mut self, id: Uuid, reason: DisconnectReason) -> anyhow::Result<()> {
        if let Some(handle) = self.subscribers.remove(&id) {
            handle
                .send_message(SessionMsg::PlaybackDisconnected(reason))
                .await?;
        }
        Ok(())
    }

    pub async fn sync(&mut self, id: Uuid, state: PlaybackState) -> anyhow::Result<()> {
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
        let mut errored_subscribers: Vec<Uuid> = vec![];
        for target in self.subscribers.values() {
            if target.id == id {
                continue;
            }
            if !send_sync_msg(target, &normalized_state).await? {
                errored_subscribers.push(target.id);
            }
            target
                .send_message(SessionMsg::PlaybackSync(
                    normalized_state.incorporate_offset(target.time_offset()),
                ))
                .await?;
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
