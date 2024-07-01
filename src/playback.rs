use std::time::SystemTime;

pub struct PlaybackState {
    playing: bool,
    time: u64,
    last_update: SystemTime,
}

impl PlaybackState {
    fn new(playing: bool, time: u64) -> Self {
        Self {
            playing,
            time,
            last_update: SystemTime::now(),
        }
    }

    fn update(&mut self, playing: bool, time: u64, timestamp: SystemTime) {
        if timestamp > self.last_update {
            self.playing = playing;
            self.time = time;
        }
    }

    #[inline]
    fn is_playing(&self) -> bool {
        self.playing
    }

    #[inline]
    fn get_time(&self) -> u64 {
        self.time
    }
}
