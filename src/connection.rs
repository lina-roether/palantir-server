use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{Error, Result};
use tokio::time::interval;

use crate::{
    api_access::{ApiAccessManager, ApiPermissions},
    listener::{ConnectionClosedReason, MessageChannel},
    messages::{Message, MessageBody},
    utils::timestamp,
};

pub struct Connection {
    channel: Arc<MessageChannel>,
    permissions: ApiPermissions,
    time_offset: Arc<AtomicI64>,
}

impl Connection {
    const TIME_OFFSET_CHECK_PERIOD: Duration = Duration::from_secs(30);

    pub async fn new(channel: MessageChannel) -> Self {
        Self {
            channel: Arc::new(channel),
            permissions: ApiPermissions::default(),
            time_offset: Arc::new(AtomicI64::new(0)),
        }
    }

    pub async fn init(&mut self, access: &ApiAccessManager) {
        if !self.authenticate(access).await {
            return;
        }

        self.start_periodic_time_offset_update();
    }

    async fn authenticate(&mut self, access: &ApiAccessManager) -> bool {
        let Some(auth_msg) = self.channel.next_response().await else {
            let future = self.channel.close(
                ConnectionClosedReason::AuthFailed,
                "No authentication message received".to_string(),
            );
            future.await;
            return false;
        };

        match auth_msg.body {
            MessageBody::ConnectionLoginV1(body) => {
                let permissions = access.get_permissions(body.api_key.as_deref());
                self.permissions = permissions;
                true
            }
            _ => {
                self.channel
                    .close(
                        ConnectionClosedReason::AuthFailed,
                        "Expected a login message".to_string(),
                    )
                    .await;
                false
            }
        }
    }

    async fn update_time_offset(channel: &MessageChannel, time_offset: &AtomicI64) -> Result<()> {
        let sent_at = timestamp();
        channel
            .send(Message::new(MessageBody::ConnectionPingV1))
            .await?;

        let Some(response) = channel.next_response().await else {
            return Err(Error::msg("No response to ping"));
        };

        match response.body {
            MessageBody::ConnectionPongV1(body) => {
                let received_at = timestamp();

                let expected_time = received_at.saturating_sub(sent_at) / 2;
                let reported_time = body.timestamp;
                let offset = ((reported_time as i128) - (expected_time as i128)) as i64;
                time_offset.store(offset, Ordering::Relaxed);
                Ok(())
            }
            _ => {
                channel
                    .send_client_error("Expected pong message".to_string())
                    .await;
                Err(Error::msg("Unexpected response to ping"))
            }
        }
    }

    fn start_periodic_time_offset_update(&self) {
        let channel = Arc::clone(&self.channel);
        let time_offset = Arc::clone(&self.time_offset);

        tokio::spawn(async move {
            let mut interval = interval(Self::TIME_OFFSET_CHECK_PERIOD);
            loop {
                Self::update_time_offset(&channel, &time_offset).await;
            }
        });
    }
}
