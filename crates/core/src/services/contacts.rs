//! ContactService — CRUD контактов.
//!
//! Handshake не реализован — пользователь вручную вводит публичный ключ собеседника
//! через UI (например, сканирует QR-код или копирует из буфера).

use uuid::Uuid;

use crate::{
    Error, Result,
    events::EventBus,
    models::contact::Contact,
    ports::storage::{CreateContact, DynStorage, UpdateContact},
};

pub struct ContactService {
    storage: DynStorage,
    events: EventBus,
}

impl ContactService {
    pub fn new(storage: DynStorage, events: EventBus) -> Self {
        Self { storage, events }
    }

    // ── CRUD ─────────────────────────────────────────────────────────────────

    /// Добавляет новый контакт со статусом `NoKey` (ключа ещё нет).
    pub async fn add_contact(
        &self,
        account_id: Uuid,
        name: String,
        email_addr: String,
        avatar: Option<Vec<u8>>,
    ) -> Result<Contact> {
        let id = Uuid::new_v4();

        self.storage
            .create_contact(CreateContact {
                id,
                account_id,
                name,
                email: email_addr.clone(),
                avatar,
            })
            .await?;

        let contact = self.storage.get_contact(id).await?;
        Ok(contact)
    }

    /// Возвращает контакт по ID.
    pub async fn get_contact(&self, id: Uuid) -> Result<Contact> {
        self.storage.get_contact(id).await
    }

    /// Список всех контактов аккаунта.
    pub async fn list_contacts(&self, account_id: Uuid) -> Result<Vec<Contact>> {
        self.storage.list_contacts(account_id).await
    }

    /// Обновляет имя и/или аватар контакта.
    pub async fn update_contact(
        &self,
        id: Uuid,
        name: Option<String>,
        avatar: Option<Option<Vec<u8>>>,
    ) -> Result<Contact> {
        self.storage
            .update_contact(id, UpdateContact { name, avatar })
            .await?;
        self.storage.get_contact(id).await
    }

    /// Удаляет контакт.
    pub async fn delete_contact(&self, id: Uuid) -> Result<()> {
        self.storage.delete_contact(id).await
    }

    // ── Ручной ввод ключей ───────────────────────────────────────────────────

    /// Сохраняет публичный ключ контакта (ручной ввод через UI).
    ///
    /// После вызова этого метода статус контакта меняется на `HasKey`
    /// и становится возможным шифрование сообщений.
    pub async fn set_contact_public_key(
        &self,
        contact_id: Uuid,
        public_keys: &encryption::keypair::PublicKeys,
    ) -> Result<()> {
        let keys_json =
            serde_json::to_string(public_keys).map_err(|e| Error::Internal(e.to_string()))?;

        self.storage
            .complete_contact_handshake(contact_id, keys_json)
            .await?;

        tracing::info!("Публичный ключ сохранён для контакта {}", contact_id);

        Ok(())
    }
}
