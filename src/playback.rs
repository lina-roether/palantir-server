use std::collections::HashMap;

use uuid::Uuid;

use crate::{
    messages::{PlaybackSourceV1, PlaybackStateV1, RoomPlaybackInfoV1},
    session::SessionHandle,
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
    fn normalize_offset(&mut self, source_offset: i64) {
        if self.playing {
            self.timestamp = self.timestamp.saturating_add_signed(-source_offset);
        }
    }

    fn incorporate_offset(&mut self, dest_offset: i64) {
        if self.playing {
            self.timestamp = self.timestamp.saturating_add_signed(dest_offset);
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

#[derive(Debug, Clone)]
pub struct Playback {
    source: PlaybackSource,
    host: SessionHandle,
    subscribers: HashMap<Uuid, SessionHandle>,
}

impl Playback {
    pub fn new(host: SessionHandle, source: PlaybackSource) -> Self {
        Self {
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
}
