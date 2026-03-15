//! AccountService — управление почтовыми аккаунтами.
//!
//! Отвечает за: добавление аккаунта, сохранение app_password в keystore,
//! генерацию identity keypair, удаление аккаунта.

use uuid::Uuid;

use crate::{
    Error, Result,
    models::account::{Account, Provider},
    ports::{
        keystore::{
            DynKeystore, SERVICE_IDENTITY, SERVICE_MAIL, app_password_key, identity_seed_key,
        },
        storage::{CreateAccount, DynStorage},
    },
};

pub struct AccountService {
    storage: DynStorage,
    keystore: DynKeystore,
}

impl AccountService {
    pub fn new(storage: DynStorage, keystore: DynKeystore) -> Self {
        Self { storage, keystore }
    }

    /// Возвращает доступ к хранилищу.
    pub fn storage(&self) -> &DynStorage {
        &self.storage
    }

    /// Добавляет новый аккаунт.
    ///
    /// 1. Проверяет что аккаунта с таким email ещё нет
    /// 2. Сохраняет app_password в keystore (не в БД!)
    /// 3. Генерирует identity keypair и сохраняет seed в keystore
    /// 4. Создаёт запись в БД
    pub async fn add_account(
        &self,
        email: String,
        app_password: String,
        provider: Provider,
    ) -> Result<Account> {
        // Проверяем дубликат
        if self.storage.get_account_by_email(&email).await.is_ok() {
            return Err(Error::AlreadyExists(format!(
                "Аккаунт {} уже добавлен",
                email
            )));
        }

        let id = Uuid::new_v4();

        // Конфигурация провайдера
        let (imap_host, imap_port, smtp_host, smtp_port) = provider_config(&provider);

        // Сохраняем app_password в keystore
        self.keystore
            .set(
                SERVICE_MAIL,
                &app_password_key(&email),
                app_password.as_bytes(),
            )
            .await?;

        // Генерируем identity keypair и сохраняем seed
        let keypair = encryption::keypair::IdentityKeypair::generate();
        let seed_bytes = keypair.seed().as_bytes();
        self.keystore
            .set(
                SERVICE_IDENTITY,
                &identity_seed_key(&id.to_string()),
                seed_bytes,
            )
            .await?;

        // Сохраняем аккаунт в БД
        self.storage
            .create_account(CreateAccount {
                id,
                email: email.clone(),
                provider: provider.clone(),
                imap_host,
                imap_port,
                smtp_host,
                smtp_port,
                echat_folder: "EChat".to_string(),
            })
            .await?;

        tracing::info!("Аккаунт {} добавлен (id={})", email, id);
        self.storage.get_account(id).await
    }

    /// Возвращает app_password из keystore.
    pub async fn get_app_password(&self, email: &str) -> Result<String> {
        let bytes = self
            .keystore
            .get(SERVICE_MAIL, &app_password_key(email))
            .await?
            .ok_or_else(|| Error::NotFound(format!("Пароль для {} не найден", email)))?;
        String::from_utf8(bytes)
            .map_err(|_| Error::Internal("Некорректный пароль в keystore".into()))
    }

    /// Загружает identity keypair из keystore.
    pub async fn load_keypair(
        &self,
        account_id: Uuid,
    ) -> Result<encryption::keypair::IdentityKeypair> {
        let seed_bytes = self
            .keystore
            .get(
                SERVICE_IDENTITY,
                &identity_seed_key(&account_id.to_string()),
            )
            .await?
            .ok_or_else(|| Error::NotFound("Identity ключ не найден".into()))?;

        let seed_arr: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| Error::Internal("Некорректный seed в keystore".into()))?;

        Ok(encryption::keypair::IdentityKeypair::from_seed(
            encryption::keypair::KeySeed::from_bytes(seed_arr),
        ))
    }

    /// Загружает identity keypair, или генерирует новый если не найден.
    pub async fn load_or_create_keypair(
        &self,
        account_id: Uuid,
    ) -> Result<encryption::keypair::IdentityKeypair> {
        // Пробуем загрузить существующий
        if let Ok(keypair) = self.load_keypair(account_id).await {
            return Ok(keypair);
        }

        // Генерируем новый и сохраняем
        let keypair = encryption::keypair::IdentityKeypair::generate();
        let seed_bytes = keypair.seed().as_bytes();
        self.keystore
            .set(
                SERVICE_IDENTITY,
                &identity_seed_key(&account_id.to_string()),
                seed_bytes,
            )
            .await?;

        tracing::info!("Identity keypair сгенерирован для аккаунта {}", account_id);
        Ok(keypair)
    }

    /// Список всех аккаунтов.
    pub async fn list_accounts(&self) -> Result<Vec<Account>> {
        self.storage.list_accounts().await
    }

    /// Удаляет аккаунт и все его данные.
    pub async fn delete_account(&self, id: Uuid) -> Result<()> {
        let account = self.storage.get_account(id).await?;

        // Удаляем секреты из keystore
        let _ = self
            .keystore
            .delete(SERVICE_MAIL, &app_password_key(&account.email))
            .await;
        let _ = self
            .keystore
            .delete(SERVICE_IDENTITY, &identity_seed_key(&id.to_string()))
            .await;

        // Удаляем из БД (CASCADE удалит всё остальное)
        self.storage.delete_account(id).await?;
        tracing::info!("Аккаунт {} удалён", account.email);
        Ok(())
    }
}

fn provider_config(provider: &Provider) -> (String, u16, String, u16) {
    match provider {
        Provider::Gmail => ("imap.gmail.com".into(), 993, "smtp.gmail.com".into(), 465),
        Provider::MailRu => ("imap.mail.ru".into(), 993, "smtp.mail.ru".into(), 465),
        Provider::Yandex => ("imap.yandex.ru".into(), 993, "smtp.yandex.ru".into(), 465),
    }
}
