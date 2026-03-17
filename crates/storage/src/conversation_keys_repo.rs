//! Репозиторий для ключей диалогов.
//!
//! Ключи хранятся на каждый диалог отдельно.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    Error, Result,
    models::{
        ConversationKeyRow, ConversationKeyStatus, NewConversationKeys, UpdateConversationKeys,
        now_iso,
    },
};

#[derive(Clone)]
pub struct ConversationKeyRepo {
    pool: SqlitePool,
}

impl ConversationKeyRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Создаёт новую запись ключей для диалога.
    pub async fn create(&self, keys: &NewConversationKeys) -> Result<()> {
        let conv_id = keys.conversation_id.to_string();
        let now = now_iso();

        sqlx::query!(
            r#"
            INSERT INTO conversation_keys
                (conversation_id, my_keypair_json, status, created_at, updated_at)
            VALUES (?, ?, 'incomplete', ?, ?)
            "#,
            conv_id,
            keys.my_keypair_json,
            now,
            now,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                Error::Conflict(format!(
                    "Ключи для диалога {} уже существуют",
                    keys.conversation_id
                ))
            } else {
                Error::Sqlx(e)
            }
        })?;

        Ok(())
    }

    /// Возвращает ключи диалога.
    pub async fn get(&self, conversation_id: Uuid) -> Result<ConversationKeyRow> {
        let conv_id = conversation_id.to_string();
        sqlx::query_as!(
            ConversationKeyRow,
            r#"
            SELECT
                conversation_id AS "conversation_id!",
                my_keypair_json,
                their_public_key_json,
                status AS "status!",
                created_at AS "created_at!",
                updated_at AS "updated_at!"
            FROM conversation_keys
            WHERE conversation_id = ?
            "#,
            conv_id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Ключи диалога {}", conversation_id)))
    }

    /// Проверяет существование ключей для диалога.
    pub async fn exists(&self, conversation_id: Uuid) -> Result<bool> {
        let conv_id = conversation_id.to_string();
        let result = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM conversation_keys WHERE conversation_id = ?)"#,
            conv_id,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(result != 0)
    }

    /// Обновляет ключи диалога.
    pub async fn update(
        &self,
        conversation_id: Uuid,
        update: &UpdateConversationKeys,
    ) -> Result<()> {
        let conv_id = conversation_id.to_string();
        let now = now_iso();

        use sqlx::QueryBuilder;
        let mut qb = QueryBuilder::new("UPDATE conversation_keys SET updated_at = ");
        qb.push_bind(&now);

        if let Some(my_keys) = &update.my_keypair_json {
            qb.push(", my_keypair_json = ");
            qb.push_bind(my_keys);
        }
        if let Some(their_key) = &update.their_public_key_json {
            qb.push(", their_public_key_json = ");
            qb.push_bind(their_key);
        }
        if let Some(status) = &update.status {
            qb.push(", status = ");
            qb.push_bind(status.as_str());
        }

        qb.push(" WHERE conversation_id = ");
        qb.push_bind(&conv_id);

        qb.build().execute(&self.pool).await?;

        Ok(())
    }

    /// Устанавливает публичный ключ собеседника и активирует ключи.
    pub async fn set_their_public_key(
        &self,
        conversation_id: Uuid,
        public_key_json: &str,
    ) -> Result<()> {
        let update = UpdateConversationKeys {
            their_public_key_json: Some(public_key_json.to_string()),
            status: Some(ConversationKeyStatus::Active),
            ..Default::default()
        };
        self.update(conversation_id, &update).await
    }

    /// Проверяет, активны ли ключи диалога.
    pub async fn is_active(&self, conversation_id: Uuid) -> Result<bool> {
        let conv_id = conversation_id.to_string();
        let result = sqlx::query_scalar!(
            r#"SELECT status FROM conversation_keys WHERE conversation_id = ?"#,
            conv_id,
        )
        .fetch_optional(&self.pool)
        .await?
        .map(|s| s == "active");
        Ok(result.unwrap_or(false))
    }

    /// Удаляет ключи диалога.
    pub async fn delete(&self, conversation_id: Uuid) -> Result<()> {
        let conv_id = conversation_id.to_string();
        let rows = sqlx::query!(
            "DELETE FROM conversation_keys WHERE conversation_id = ?",
            conv_id
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows == 0 {
            return Err(Error::NotFound(format!(
                "Ключи диалога {}",
                conversation_id
            )));
        }
        Ok(())
    }
}
