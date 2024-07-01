pub enum UserRole {
    Host,
    Guest,
}

pub struct User {
    name: String,
    key: Box<[u8]>,
    role: UserRole,
    time_offset: i32,
}
