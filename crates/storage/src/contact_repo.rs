//! Репозиторий контактов.
//!
//! Контакт принадлежит аккаунту. Публичные ключи хранятся как JSON —
//! это позволяет менять формат ключей без миграции схемы.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    Error, Result,
    models::{ContactRow, NewContact, UpdateContact, now_iso},
};

#[derive(Clone)]
pub struct ContactRepo {
    pool: SqlitePool,
}

impl ContactRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Создаёт новый контакт.
    pub async fn create(&self, contact: &NewContact) -> Result<()> {
        let id = contact.id.to_string();
        let account_id = contact.account_id.to_string();
        let now = now_iso();

        sqlx::query!(
            r#"
            INSERT INTO contacts
                (id, account_id, name, email, avatar, status, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, 'unregistered', ?, ?)
            "#,
            id,
            account_id,
            contact.name,
            contact.email,
            contact.avatar,
            now,
            now,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                Error::Conflict(format!(
                    "Контакт {} уже существует в аккаунте",
                    contact.email
                ))
            } else {
                Error::Sqlx(e)
            }
        })?;

        Ok(())
    }

    /// Возвращает контакт по ID.
    pub async fn get_by_id(&self, id: Uuid) -> Result<ContactRow> {
        let id_str = id.to_string();
        sqlx::query_as!(
            ContactRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", name AS "name!", email AS "email!", avatar, status AS "status!", public_keys_json, handshake_at, created_at AS "created_at!", updated_at AS "updated_at!" FROM contacts WHERE id = ?"#,
            id_str,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Контакт {}", id)))
    }

    /// Возвращает контакт по email в рамках аккаунта.
    pub async fn get_by_email(&self, account_id: Uuid, email: &str) -> Result<ContactRow> {
        let account_id_str = account_id.to_string();
        sqlx::query_as!(
            ContactRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", name AS "name!", email AS "email!", avatar, status AS "status!", public_keys_json, handshake_at, created_at AS "created_at!", updated_at AS "updated_at!" FROM contacts WHERE account_id = ? AND email = ?"#,
            account_id_str,
            email,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Контакт {} в аккаунте {}", email, account_id)))
    }

    /// Возвращает все контакты аккаунта, сортировка по имени.
    pub async fn list(&self, account_id: Uuid) -> Result<Vec<ContactRow>> {
        let account_id_str = account_id.to_string();
        Ok(sqlx::query_as!(
            ContactRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", name AS "name!", email AS "email!", avatar, status AS "status!", public_keys_json, handshake_at, created_at AS "created_at!", updated_at AS "updated_at!" FROM contacts WHERE account_id = ? ORDER BY name COLLATE NOCASE"#,
            account_id_str,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Возвращает только активные контакты (завершён handshake).
    pub async fn list_active(&self, account_id: Uuid) -> Result<Vec<ContactRow>> {
        let account_id_str = account_id.to_string();
        Ok(sqlx::query_as!(
            ContactRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", name AS "name!", email AS "email!", avatar, status AS "status!", public_keys_json, handshake_at, created_at AS "created_at!", updated_at AS "updated_at!" FROM contacts WHERE account_id = ? AND status = 'active' ORDER BY name COLLATE NOCASE"#,
            account_id_str,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Обновляет поля контакта (только те что переданы в UpdateContact).
    pub async fn update(&self, id: Uuid, update: &UpdateContact) -> Result<()> {
        let id_str = id.to_string();
        let now = now_iso();

        // Строим запрос динамически — обновляем только переданные поля
        // sqlx не поддерживает динамические UPDATE удобно, поэтому вручную
        // Позиционные параметры собираем отдельно
        // и передаём через query_builder

        use sqlx::QueryBuilder;
        let mut qb = QueryBuilder::new("UPDATE contacts SET updated_at = ");
        qb.push_bind(&now);

        if let Some(name) = &update.name {
            qb.push(", name = ");
            qb.push_bind(name);
        }
        if let Some(avatar_opt) = &update.avatar {
            qb.push(", avatar = ");
            qb.push_bind(avatar_opt.as_deref());
        }
        if let Some(status) = &update.status {
            qb.push(", status = ");
            qb.push_bind(status.as_str());
        }
        if let Some(keys) = &update.public_keys_json {
            qb.push(", public_keys_json = ");
            qb.push_bind(keys);
        }

        qb.push(" WHERE id = ");
        qb.push_bind(&id_str);

        qb.build().execute(&self.pool).await?;

        Ok(())
    }

    /// Обновляет публичные ключи и статус после завершения handshake.
    pub async fn complete_handshake(&self, id: Uuid, public_keys_json: &str) -> Result<()> {
        let id_str = id.to_string();
        let now = now_iso();

        sqlx::query!(
            r#"UPDATE contacts
               SET status = 'active',
                   public_keys_json = ?,
                   handshake_at = ?,
                   updated_at = ?
               WHERE id = ?"#,
            public_keys_json,
            now,
            now,
            id_str,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Обновляет статус контакта на pending (handshake отправлен).
    pub async fn set_pending(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let now = now_iso();

        sqlx::query!(
            "UPDATE contacts SET status = 'pending', updated_at = ? WHERE id = ?",
            now,
            id_str,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Удаляет контакт.
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let rows = sqlx::query!("DELETE FROM contacts WHERE id = ?", id_str)
            .execute(&self.pool)
            .await?
            .rows_affected();

        if rows == 0 {
            return Err(Error::NotFound(format!("Контакт {}", id)));
        }
        Ok(())
    }
}
