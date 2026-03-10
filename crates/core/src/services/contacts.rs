//! ContactService — CRUD контактов и запуск handshake.

use uuid::Uuid;

use crate::{
    events::{ChatEvent, EventBus},
    models::contact::{Contact, ContactStatus},
    ports::{
        email::{DynEmailTransport, OutgoingEmail},
        storage::{CreateContact, DynStorage, UpdateContact},
    },
    services::account::AccountService,
    Error, Result,
};

pub struct ContactService {
    storage: DynStorage,
    email: DynEmailTransport,
    account_svc: AccountService,
    events: EventBus,
}

impl ContactService {
    pub fn new(
        storage: DynStorage,
        email: DynEmailTransport,
        account_svc: AccountService,
        events: EventBus,
    ) -> Self {
        Self { storage, email, account_svc, events }
    }

    // ── CRUD ─────────────────────────────────────────────────────────────────

    /// Добавляет новый контакт и инициирует handshake.
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

        // Сразу пытаемся отправить handshake
        // Если контакт без приложения — отправим invite
        self.initiate_handshake(account_id, id).await?;

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

    // ── Handshake ────────────────────────────────────────────────────────────

    /// Инициирует handshake с контактом.
    ///
    /// - Если контакт имеет приложение → отправляет HandshakeInit-письмо
    /// - Если нет → отправляет invite с зашифрованным payload в теле
    pub async fn initiate_handshake(
        &self,
        account_id: Uuid,
        contact_id: Uuid,
    ) -> Result<()> {
        let account = self.storage.get_account(account_id).await?;
        let contact = self.storage.get_contact(contact_id).await?;

        // Загружаем keypair
        let keypair = self.account_svc.load_keypair(account_id).await?;

        // Формируем handshake payload
        let handshake_msg =
            encryption::handshake::HandshakeMessage::new_init(&keypair, &account.email);
        let handshake_b64 = handshake_msg.to_base64()?;

        // Строим письмо через codec
        let outgoing = OutgoingEmail {
            from: account.email.clone(),
            to: vec![contact.email.clone()],
            subject: encryption::disguise::random_subject(),
            body: handshake_b64,
            extra_headers: vec![("X-EChat".into(), "1".into())],
        };

        // Пробуем отправить
        match self.email.send(outgoing).await {
            Ok(()) => {
                self.storage.set_contact_pending(contact_id).await?;
                tracing::info!(
                    "Handshake отправлен: {} → {}",
                    account.email,
                    contact.email
                );
            }
            Err(e) => {
                tracing::warn!("Не удалось отправить handshake: {}. Статус остаётся unregistered.", e);
                // Не возвращаем ошибку — статус останется unregistered,
                // SyncEngine повторит при следующей синхронизации
            }
        }

        Ok(())
    }

    /// Обрабатывает входящий HandshakeAck от контакта.
    ///
    /// Вызывается из `sync::processor` когда приходит письмо с Ack.
    pub async fn handle_handshake_ack(
        &self,
        account_id: Uuid,
        from_email: &str,
        their_public_keys: &encryption::keypair::PublicKeys,
    ) -> Result<()> {
        let contact = self
            .storage
            .get_contact_by_email(account_id, from_email)
            .await?;

        // Сохраняем публичные ключи как JSON
        let keys_json = serde_json::to_string(their_public_keys)
            .map_err(|e| Error::Internal(e.to_string()))?;

        self.storage
            .complete_contact_handshake(contact.id, keys_json)
            .await?;

        tracing::info!("Handshake завершён с {}", from_email);

        // Уведомляем UI
        self.events.emit(ChatEvent::ContactActivated {
            contact_id: contact.id,
            email: from_email.to_string(),
        });

        Ok(())
    }

    /// Обрабатывает входящий HandshakeInit — отвечает Ack.
    pub async fn handle_handshake_init(
        &self,
        account_id: Uuid,
        from_email: &str,
        their_public_keys: &encryption::keypair::PublicKeys,
    ) -> Result<()> {
        let account = self.storage.get_account(account_id).await?;

        // Ищем контакт или создаём автоматически
        let contact = match self.storage.get_contact_by_email(account_id, from_email).await {
            Ok(c) => c,
            Err(_) => {
                // Автоматически добавляем нового контакта
                let id = Uuid::new_v4();
                self.storage
                    .create_contact(CreateContact {
                        id,
                        account_id,
                        name: from_email.to_string(),
                        email: from_email.to_string(),
                        avatar: None,
                    })
                    .await?;
                self.storage.get_contact(id).await?
            }
        };

        // Сохраняем публичные ключи
        let keys_json = serde_json::to_string(their_public_keys)
            .map_err(|e| Error::Internal(e.to_string()))?;
        self.storage
            .complete_contact_handshake(contact.id, keys_json)
            .await?;

        // Отправляем Ack
        let keypair = self.account_svc.load_keypair(account_id).await?;
        let ack_msg = encryption::handshake::HandshakeMessage::new_ack(&keypair, &account.email);
        let ack_b64 = ack_msg.to_base64()?;

        self.email
            .send(OutgoingEmail {
                from: account.email.clone(),
                to: vec![from_email.to_string()],
                subject: encryption::disguise::random_subject(),
                body: ack_b64,
                extra_headers: vec![("X-EChat".into(), "1".into())],
            })
            .await?;

        tracing::info!("Handshake Ack отправлен → {}", from_email);

        self.events.emit(ChatEvent::ContactActivated {
            contact_id: contact.id,
            email: from_email.to_string(),
        });

        Ok(())
    }
}
