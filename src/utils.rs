use std::time::{SystemTime, UNIX_EPOCH};

pub fn timestamp() -> u64 {
    let duration_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time too far in the past");

    duration_since_epoch
        .as_millis()
        .try_into()
        .expect("System time too far in the future")
}
