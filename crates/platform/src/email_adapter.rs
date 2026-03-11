//! EmailAdapter — оборачивает `email::EmailClient` в трейт `core::ports::EmailTransport`.
//!
//! Конвертирует типы:
//! ```text
//! core::ports::email::OutgoingEmail  →  email::types::OutgoingMessage
//! email::types::IncomingMessage      →  core::ports::email::IncomingEmail
//! ```

use std::sync::Arc;

use async_trait::async_trait;

use echat_core::{
    Error, Result,
    ports::email::{EmailTransport, IncomingEmail, OutgoingEmail},
};

/// Адаптер: `core::EmailTransport` поверх `email::EmailClient`.
pub struct EmailAdapter {
    client: email::EmailClient,
}

impl EmailAdapter {
    pub fn new(client: email::EmailClient) -> Arc<Self> {
        Arc::new(Self { client })
    }
}

#[async_trait]
impl EmailTransport for EmailAdapter {
    async fn send(&self, email: OutgoingEmail) -> Result<()> {
        let msg = email::types::OutgoingMessage {
            from: email.from,
            to: email.to,
            subject: email.subject,
            body: email.body,
            extra_headers: email.extra_headers,
        };
        self.client
            .send_message(msg)
            .await
            .map_err(|e| Error::Transport(e.to_string()))
    }

    async fn fetch_new(&self, since_uid: Option<u32>) -> Result<Vec<IncomingEmail>> {
        let since = since_uid.map(email::types::MessageUid);

        let messages = self
            .client
            .fetch_new_messages(since)
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        Ok(messages.into_iter().map(incoming_to_core).collect())
    }

    async fn idle_wait(&self) -> Result<bool> {
        self.client
            .idle_wait()
            .await
            .map_err(|e| Error::Transport(e.to_string()))
    }

    async fn delete_messages(&self, folder: &str, uids: &[u32]) -> Result<()> {
        let uid_list: Vec<email::types::MessageUid> =
            uids.iter().map(|&u| email::types::MessageUid(u)).collect();
        self.client
            .delete_messages(folder, &uid_list)
            .await
            .map_err(|e| Error::Transport(e.to_string()))
    }

    async fn ensure_echat_folder(&self) -> Result<()> {
        self.client
            .ensure_echat_folder()
            .await
            .map_err(|e| Error::Transport(e.to_string()))
    }
}

// ── Конвертация типов ─────────────────────────────────────────────────────────

fn incoming_to_core(msg: email::types::IncomingMessage) -> IncomingEmail {
    IncomingEmail {
        uid: msg.uid.0,
        folder: msg.folder,
        from: msg.from,
        to: msg.to,
        subject: msg.subject,
        body: msg.body,
        headers: msg.headers.0,
        date: msg.date,
    }
}
