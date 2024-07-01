use crate::playback::PlaybackState;

pub struct Media {
    page_href: String,
    frame_href: String,
    element_query: String,
    state: PlaybackState,
}
