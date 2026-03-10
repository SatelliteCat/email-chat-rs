//! GroupService — создание групп, управление участниками, групповая рассылка.

use uuid::Uuid;

use crate::{
    events::{ChatEvent, EventBus},
    models::conversation::GroupRole,
    ports::{
        email::{DynEmailTransport, OutgoingEmail},
        storage::DynStorage,
    },
    services::account::AccountService,
    Error, Result,
};

pub struct GroupService {
    storage: DynStorage,
    email: DynEmailTransport,
    account_svc: AccountService,
    events: EventBus,
}

impl GroupService {
    pub fn new(
        storage: DynStorage,
        email: DynEmailTransport,
        account_svc: AccountService,
        events: EventBus,
    ) -> Self {
        Self { storage, email, account_svc, events }
    }

    /// Создаёт новый групповой чат.
    ///
    /// `member_ids` — список contact_id участников (не включая себя).
    /// Все участники должны быть в статусе Active (завершённый handshake).
    pub async fn create_group(
        &self,
        account_id: Uuid,
        name: String,
        avatar: Option<Vec<u8>>,
        member_ids: Vec<Uuid>,
    ) -> Result<Uuid> {
        // Проверяем что все участники Active
        let mut members_with_roles: Vec<(Uuid, GroupRole, Option<String>)> = Vec::new();

        for contact_id in &member_ids {
            let contact = self.storage.get_contact(*contact_id).await?;
            if contact.status != crate::models::contact::ContactStatus::Active {
                return Err(Error::InvalidState(format!(
                    "Контакт {} не завершил handshake",
                    contact.email
                )));
            }
            let snapshot = contact
                .public_keys
                .as_ref()
                .map(|k| serde_json::to_string(k).unwrap_or_default());
            members_with_roles.push((*contact_id, GroupRole::Member, snapshot));
        }

        let conv_id = Uuid::new_v4();
        self.storage
            .create_group_conversation(
                conv_id,
                account_id,
                name.clone(),
                avatar,
                members_with_roles,
            )
            .await?;

        // Рассылаем GroupCreate уведомление всем участникам
        self.broadcast_group_event(
            account_id,
            conv_id,
            &member_ids,
            GroupEventKind::Created { name },
        )
        .await?;

        tracing::info!("Группа {} создана (id={})", conv_id, conv_id);
        Ok(conv_id)
    }

    /// Добавляет участника в группу.
    /// Требует роли Owner или Admin.
    pub async fn add_member(
        &self,
        account_id: Uuid,
        conv_id: Uuid,
        new_contact_id: Uuid,
        requester_contact_id: Option<Uuid>,
    ) -> Result<()> {
        // Проверяем права если указан инициатор
        if let Some(req_id) = requester_contact_id {
            self.check_member_permission(conv_id, req_id, "добавлять участников").await?;
        }

        let contact = self.storage.get_contact(new_contact_id).await?;
        if contact.status != crate::models::contact::ContactStatus::Active {
            return Err(Error::InvalidState(format!(
                "Контакт {} не завершил handshake",
                contact.email
            )));
        }

        let snapshot = contact
            .public_keys
            .as_ref()
            .map(|k| serde_json::to_string(k).unwrap_or_default());

        self.storage
            .add_group_member(conv_id, new_contact_id, GroupRole::Member, snapshot)
            .await?;

        // Уведомляем группу
        let all_members = self.get_member_ids(conv_id).await?;
        self.broadcast_group_event(
            account_id,
            conv_id,
            &all_members,
            GroupEventKind::MemberAdded { contact_id: new_contact_id },
        )
        .await?;

        self.events.emit(ChatEvent::GroupMemberAdded {
            conversation_id: conv_id,
            contact_id: new_contact_id,
        });

        Ok(())
    }

    /// Удаляет участника из группы.
    pub async fn remove_member(
        &self,
        account_id: Uuid,
        conv_id: Uuid,
        contact_id: Uuid,
        requester_contact_id: Option<Uuid>,
    ) -> Result<()> {
        if let Some(req_id) = requester_contact_id {
            self.check_member_permission(conv_id, req_id, "удалять участников").await?;
        }

        self.storage.remove_group_member(conv_id, contact_id).await?;

        // Уведомляем оставшихся участников
        let remaining = self.get_member_ids(conv_id).await?;
        self.broadcast_group_event(
            account_id,
            conv_id,
            &remaining,
            GroupEventKind::MemberRemoved { contact_id },
        )
        .await?;

        self.events.emit(ChatEvent::GroupMemberRemoved {
            conversation_id: conv_id,
            contact_id,
        });

        Ok(())
    }

    /// Отправляет зашифрованное сообщение всем участникам группы.
    pub async fn send_group_message(
        &self,
        account_id: Uuid,
        conv_id: Uuid,
        body: String,
    ) -> Result<()> {
        let account = self.storage.get_account(account_id).await?;
        let keypair = self.account_svc.load_keypair(account_id).await?;
        let members_db = self.storage.get_group_members(conv_id).await?;

        // Собираем GroupMember для fan-out шифрования
        let mut group_members: Vec<encryption::group::GroupMember> = Vec::new();
        let mut recipient_emails: Vec<String> = Vec::new();

        for m in &members_db {
            let contact = self.storage.get_contact(m.contact_id).await?;
            if let Some(keys) = &contact.public_keys {
                if let Some(x25519) = keys.x25519_bytes() {
                    group_members.push(encryption::group::GroupMember {
                        email: contact.email.clone(),
                        public_key_bytes: x25519,
                    });
                    recipient_emails.push(contact.email);
                }
            }
        }

        // Добавляем себя (чтобы прочитать своё сообщение)
        let my_pubkey = keypair.public_key();
        group_members.push(encryption::group::GroupMember {
            email: account.email.clone(),
            public_key_bytes: *my_pubkey.as_bytes(),
        });

        // Fan-out шифрование
        let msg_id = Uuid::new_v4();
        let envelope = serde_json::json!({
            "msg_id": msg_id,
            "conv_id": conv_id,
            "kind": "text",
            "sent_at": Utc::now().to_rfc3339(),
            "body": body,
            "protocol_version": 1,
        });
        let envelope_bytes = serde_json::to_vec(&envelope)
            .map_err(|e| Error::Internal(e.to_string()))?;

        let payload = encryption::group::encrypt(
            &envelope_bytes,
            &keypair,
            &conv_id.to_string(),
            &group_members,
        )
        .map_err(|e| Error::Encryption(e.to_string()))?;

        let payload_b64 = payload.to_base64()
            .map_err(|e| Error::Encryption(e.to_string()))?;

        // Отправляем одно письмо на всех (To: + CC:)
        self.email
            .send(OutgoingEmail {
                from: account.email,
                to: recipient_emails,
                subject: encryption::disguise::random_subject(),
                body: payload_b64,
                extra_headers: vec![("X-EChat".into(), "1".into())],
            })
            .await?;

        Ok(())
    }

    // ── Внутренние ───────────────────────────────────────────────────────────

    async fn get_member_ids(&self, conv_id: Uuid) -> Result<Vec<Uuid>> {
        Ok(self
            .storage
            .get_group_members(conv_id)
            .await?
            .into_iter()
            .map(|m| m.contact_id)
            .collect())
    }

    async fn check_member_permission(
        &self,
        conv_id: Uuid,
        contact_id: Uuid,
        action: &str,
    ) -> Result<()> {
        let members = self.storage.get_group_members(conv_id).await?;
        let member = members
            .iter()
            .find(|m| m.contact_id == contact_id)
            .ok_or_else(|| Error::NotFound("Участник не найден".into()))?;

        if !member.role.can_add_members() {
            return Err(Error::Forbidden(format!(
                "Недостаточно прав для: {}",
                action
            )));
        }
        Ok(())
    }

    async fn broadcast_group_event(
        &self,
        account_id: Uuid,
        conv_id: Uuid,
        member_ids: &[Uuid],
        _kind: GroupEventKind,
    ) -> Result<()> {
        // TODO: в будущем — отправить GroupKeyUpdate письма участникам
        // Пока просто логируем
        tracing::debug!(
            "Group event для беседы {} → {} участников",
            conv_id,
            member_ids.len()
        );
        Ok(())
    }
}

use chrono::Utc;

enum GroupEventKind {
    Created { name: String },
    MemberAdded { contact_id: Uuid },
    MemberRemoved { contact_id: Uuid },
}
