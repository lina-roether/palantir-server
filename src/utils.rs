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

#[macro_export]
macro_rules! id_type {
    ($name: ident $(, $derive:ident)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash $(, $derive)*)]
        pub struct $name(::uuid::Uuid);

        #[allow(unused)]
        impl $name {
            fn new() -> Self {
                Self(::uuid::Uuid::new_v4())
            }
        }

        impl From<::uuid::Uuid> for $name {
            fn from(val: ::uuid::Uuid) -> Self {
                Self(val)
            }
        }

        impl ::std::ops::Deref for $name {
            type Target = ::uuid::Uuid;

            fn deref(&self) -> &::uuid::Uuid {
                &self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}
