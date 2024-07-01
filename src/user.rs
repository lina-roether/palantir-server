pub enum UserRole {
    Host,
    Spectator,
}

pub struct User {
    name: String,
    key: Box<[u8]>,
    role: UserRole,
    time_offset: i32,
}
