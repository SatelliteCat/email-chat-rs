# EChat — Шаги реализации

> Пошаговый план разработки от нуля до готового приложения.  
> Каждый шаг завершён когда компилируется и проходят тесты.

---

## Статус

| Шаг | Крейт / Модуль | Статус |
|---|---|---|
| 1 | `crates/encryption` | ✅ Завершён |
| 2 | `crates/email` | ✅ Завершён |
| 3 | `crates/storage` | ✅ Завершён |
| 4 | `crates/core` | ✅ Завершён |
| 5 | `crates/platform` | ✅ Завершён |
| 6 | `apps/desktop` | ✅ Завершён |
| 7 | `apps/mobile` | ✅ Завершён (stub keystore) |
| 8 | Android Keystore JNI | ⏳ Следующий шаг |
| 9 | iOS Keychain bridge | ⏳ Следующий шаг |
| 10 | Финальная интеграция | ⏳ |

---

## Шаг 1 — `crates/encryption`

**Цель:** вся криптография в одном крейте, без сети, без IO.

### Зависимости Cargo.toml

```toml
x25519-dalek       = { version = "2", features = ["static_secrets"] }
ed25519-dalek      = { version = "2", features = ["rand_core"] }
chacha20poly1305   = "0.10"
hkdf               = "0.12"
sha2               = "0.10"
argon2             = "0.5"
rand               = "0.8"
bip39              = "2"
zeroize            = { version = "1", features = ["derive"] }
base64             = "0.22"
thiserror          = "1"
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
hex                = "0.4"
```

### Порядок реализации файлов

1. **`lib.rs`** — `Error` enum, `Result<T>`, prelude
2. **`keypair.rs`** — `KeySeed` (zeroize), `IdentityKeypair`, `PublicKeys`
   - `generate()` → OsRng seed
   - `from_seed(seed)` → X25519 secret + Ed25519 через SHA-256 деривацию
   - `sign(msg)` / `verify(msg, sig, pubkey)`
3. **`session.rs`** — `SharedSecret` (zeroize)
   - `derive_shared_secret(my_secret, their_public, context)`
   - X25519 → HKDF-SHA256 → 32 байта
   - Отклонять нулевой DH-результат
4. **`cipher.rs`** — `EncryptedPayload`
   - `encrypt(plaintext, &SharedSecret)` — случайный nonce через OsRng
   - `decrypt(payload, &SharedSecret)` → Err при подделке
   - `to_base64()` / `from_base64()` — magic bytes `[0xEC, 0xC4, 0xA7, 0x01]` в начале
   - `has_magic_prefix(bytes)` — быстрая проверка
5. **`handshake.rs`** — `HandshakeMessage`
   - Поля: version, kind (Init/Ack), public_keys, signature, timestamp_secs, from_email
   - `sign_and_create()` — подпись: `Ed25519(x25519_pub || ed25519_pub || ts_le)`
   - `verify(max_age_secs)` — подпись + replay protection
6. **`group.rs`** — `GroupEncryptedPayload`
   - `encrypt(plaintext, sender, group_id, members: &[GroupMember])`
   - `decrypt(payload, recipient_email, recipient_keypair, sender_pubkey, group_id)`
   - Zeroize `session_key` после использования
7. **`export.rs`** — резервное копирование
   - `export_keypair(keypair, password, format)` → `ExportedKey`
   - `import_keypair(exported, password)` → `IdentityKeypair`
   - BIP-39: 24 слова из 32 байт seed
   - Argon2id: 64MB / 3 итерации / 4 параллелизма
   - Checksum: `hex(SHA-256(seed)[0..4])`
8. **`disguise.rs`** — маскировка
   - 16 предустановленных тем на русском
   - `build_email(payload_b64, kind)` → `DisguisedEmail`
   - `is_echat_message(headers, body)` — X-EChat header или magic bytes
   - `extract_payload(body)` — обработка invite формата с `---`
9. **`tests/integration.rs`** — 3 теста: direct chat, group chat, export/import

### Критерий готовности

```bash
cargo test -p encryption
# Все тесты зелёные, нет panic
```

---

## Шаг 2 — `crates/email`

**Цель:** рабочий IMAP/SMTP клиент для mail.ru и yandex.ru.

### Зависимости

```toml
encryption  = { path = "../encryption" }
async-imap  = { version = "0.9", features = ["runtime-tokio"] }
async-native-tls = "0.5"
lettre       = { version = "0.11", default-features = false,
                 features = ["builder","tokio1","tokio1-native-tls","smtp-transport"] }
mail-parser  = "0.9"
tokio        = { version = "1", features = ["full"] }
tokio-native-tls = "0.3"
native-tls   = "0.2"
base64       = "0.22"
```

### Порядок реализации

1. **`providers/mod.rs`** — `ProviderConfig`, `ImapConfig`, `SmtpConfig`
   - `::mailru(email, pass)` — imap.mail.ru:993 / smtp.mail.ru:465
   - `::yandex(email, pass)` — imap.yandex.ru:993 / smtp.yandex.ru:465
   - `::detect(email, pass)` — автоопределение по домену
   - Все домены mail.ru: mail.ru, inbox.ru, list.ru, bk.ru
   - Все домены yandex: yandex.ru, ya.ru, yandex.com, yandex.kz, yandex.by, yandex.ua
2. **`types.rs`** — `MessageUid(u32)`, `IncomingMessage`, `OutgoingMessage`, `RawEmailHeaders`
3. **`imap/mod.rs`** — `ImapConnection`
   - TLS connect: `async_native_tls::TlsConnector` → `async_imap::Client::new()`
   - `LOGIN` с app_password
   - `SELECT "EChat"` перед IDLE
   - `fetch_new(since_uid)` — `UID FETCH {uid}:* (RFC822 UID FLAGS)`
   - `idle_once()` — 29 минут таймаут, возвращает `bool` (были ли изменения)
   - `delete_messages(uids)` — STORE FLAGS + EXPUNGE
   - `ensure_folder()` — CREATE если не существует, игнорировать `[ALREADYEXISTS]`
   - Автореконнект при разрыве
4. **`smtp/mod.rs`** — `SmtpConnection`
   - `lettre::AsyncSmtpTransport::relay(host)?.port(465).tls(Tls::Wrapper(...))`
   - Новый транспорт на каждую отправку (stateless)
5. **`codec.rs`** — `ChatEnvelope` сериализация/десериализация
   - `encode_message(from, to, payload_b64)` → `OutgoingMessage`
   - `encode_handshake(...)` / `encode_invite(...)`
6. **`lib.rs`** — `EmailClient` facade
   - `connect(config)` → `EmailClient`
   - Делегирует в `ImapConnection` + `SmtpConnection`

### Критерий готовности

```bash
# Unit тесты
cargo test -p email

# Live тест (опционально)
ECHAT_TEST_EMAIL=user@mail.ru ECHAT_TEST_PASS=app_password \
    cargo test -p email -- --ignored --nocapture
```

---

## Шаг 3 — `crates/storage`

**Цель:** SQLite с compile-time проверкой запросов, 4 таблицы, 4 репозитория.

### Зависимости

```toml
sqlx = { version = "0.8", default-features = false,
         features = ["runtime-tokio","tls-native-tls","sqlite",
                     "macros","migrate","chrono","uuid"] }
```

### Порядок реализации

1. **Миграции** в `src/migrations/`:
   - `001_accounts.sql`
   - `002_contacts.sql` — `UNIQUE(account_id, email)`
   - `003_conversations.sql` — conversations + group_members
   - `004_messages.sql` — messages + индексы по (conv_id, sent_at), (conv_id, imap_uid)

2. **`lib.rs`** — `Database`
   - `open(path)` — создать директорию, `SqlitePoolOptions`, WAL+NORMAL+FK pragma, `sqlx::migrate!()`
   - `open_in_memory()` — для тестов
   - Методы-фабрики: `accounts()`, `contacts()`, `conversations()`, `messages()`

3. **`models.rs`** — row-структуры (`AccountRow`, `ContactRow`, ...) + `NewAccount`, `UpdateContact`, enums

4. **Репозитории** (каждый принимает `&SqlitePool`):
   - `AccountRepo`: create, get_by_id, get_by_email, list, update_sync_state, delete
   - `ContactRepo`: create (возвращает Conflict при дублировании email), get_by_id, get_by_email, list, update, set_pending, complete_handshake, delete
   - `ConversationRepo`: create_direct, create_group, get_by_id, find_direct, list, get_members, update_last_message, mark_as_read, add_member, remove_member, delete (CASCADE)
   - `MessageRepo`: create (Conflict при дублировании id), exists, get_history (cursor), update_status, get_imap_uids_for_deletion, delete_conversation_messages, get_queued

5. **`tests/integration.rs`** — 14 тестов через `Database::open_in_memory()`

### Тонкости

```bash
# ОБЯЗАТЕЛЬНО перед cargo build/test
export DATABASE_URL="sqlite::memory:"
```

Каскадное удаление: удаление аккаунта → contacts → conversations → messages через `ON DELETE CASCADE`.

### Критерий готовности

```bash
DATABASE_URL="sqlite::memory:" cargo test -p storage
# 14 тестов зелёные
```

---

## Шаг 4 — `crates/core`

**Цель:** бизнес-логика независимая от конкретных реализаций. Только трейты и сервисы.

### Зависимости

```toml
encryption  = { path = "../encryption" }
tokio       = { version = "1", features = ["full"] }
async-trait = "0.1"
```

### Порядок реализации

1. **`src/ports/`** — трейты:
   - `email.rs` — `EmailTransport` + `IncomingEmail` / `OutgoingEmail`
   - `storage.rs` — `StoragePort` + все DTO (CreateAccount, CreateContact, ...)
   - `keystore.rs` — `KeystorePort` + `InMemoryKeystore` + константы + key-name helpers

2. **`src/models/`** — доменные модели (без sqlx, без uuid напрямую):
   - `account.rs` — `Account`, `Provider`
   - `contact.rs` — `Contact`, `ContactStatus`, `ContactPublicKeys`
   - `conversation.rs` — `Conversation`, `ConversationKind`, `GroupMember`, `GroupRole`
   - `message.rs` — `Message`, `MessageKind`, `MessageStatus`, `Message::is_incoming(my_email)`

3. **`src/events.rs`** — `ChatEvent` enum + `EventBus` (`broadcast::Sender<ChatEvent>`)

4. **`src/services/`**:
   - `account.rs` — `AccountService::add_account()` — сохранить в keystore + создать в БД
   - `contacts.rs` — `ContactService::add_contact()` → `initiate_handshake()` → отправить email
   - `chat.rs` — `ChatService::send_message()` — зашифровать или поставить в очередь; `delete_conversation(with_server: bool)` — EXPUNGE + DB
   - `group.rs` — `GroupService::create_group()` — все участники должны быть Active

5. **`src/sync/`**:
   - `processor.rs` — `process_incoming(msg)` — по magic bytes определить тип, маршрутизировать
   - `engine.rs` — `SyncEngine::start()` → tokio::spawn IDLE loop, экспоненциальный reconnect

6. **`src/lib.rs`** — `AppConfig`, `AppState` (DI через конструктор)

7. **`tests/integration.rs`** — 12 тестов: EventBus, InMemoryKeystore, GroupRole, key names

### Критерий готовности

```bash
cargo test -p core
# 12 тестов зелёные
```

---

## Шаг 5 — `crates/platform`

**Цель:** соединить трейты core с конкретными реализациями. Keystore для каждой ОС.

### Зависимости

```toml
core     = { path = "../core" }
email    = { path = "../email" }
storage  = { path = "../storage" }

[target.'cfg(not(target_os = "android"))'.dependencies]
keyring = { version = "3", features = ["tokio"] }

[dev-dependencies]
hex  = "0.4"
rand = "0.8"
```

### Порядок реализации

1. **`src/keystore/mod.rs`** — cfg-based реэкспорт `PlatformKeystore`

2. **`src/keystore/desktop.rs`** — `DesktopKeystore`
   - `set()` → base64(bytes) → `keyring::Entry::set_password()`
   - `get()` → base64 decode → `Vec<u8>`
   - `delete()` → игнорировать `NoEntry`
   - Все вызовы через `spawn_blocking`

3. **`src/keystore/android.rs`** — stub с TODO комментариями для JNI

4. **`src/keystore/ios.rs`** — stub с TODO комментариями для Swift bridge

5. **`src/email_adapter.rs`** — `impl EmailTransport for EmailAdapter`
   - Конвертация `OutgoingEmail → OutgoingMessage` и обратно
   - Маппинг ошибок `email::Error → core::Error::Transport`

6. **`src/storage_adapter.rs`** — `impl StoragePort for StorageAdapter`
   - Явные функции конвертации `to_core_account()`, `to_core_contact()`, etc.
   - Маппинг `storage::Error::NotFound → core::Error::NotFound`

7. **`src/lib.rs`** — `build_app_state(email, password, db_path, config) → AppState`

8. **`tests/integration.rs`** — 17 тестов через in-memory SQLite

### Критерий готовности

```bash
DATABASE_URL="sqlite::memory:" cargo test -p platform
# 17 тестов зелёные
```

---

## Шаг 6 — `apps/desktop`

**Цель:** рабочее desktop-приложение на egui.

### Зависимости

```toml
platform    = { path = "../../crates/platform" }
eframe      = { version = "0.29", features = ["persistence"] }
egui        = "0.29"
dirs        = "5"
```

### Порядок реализации

1. **`src/state.rs`** — `UiState`, `Screen`, `LoginState`, `ConversationItem`, `ChatUiState`, `Toast`

2. **`src/runtime.rs`** — `AppEvent` enum (все варианты), `AsyncRuntime`, `EventSender`
   - `AsyncRuntime::new()` — tokio multi-thread runtime
   - `spawn(future)` — делегирует в rt
   - `event_sender()` — cloneable sender с `ctx.request_repaint()`
   - `subscribe_to_core_events()` — слушает EventBus, пишет в AppEvent

3. **`src/views/mod.rs`** — `theme` модуль: константы цветов + `apply(ctx)`

4. **`src/views/login.rs`** — форма логина
   - Email + password fields, eye-кнопка, spinner, блок ошибки
   - Enter в любом поле → отправить

5. **`src/views/sidebar.rs`** — список бесед
   - Аватары (circle + буква), время, badge непрочитанных
   - Строка поиска, индикатор sync
   - Возвращает `Option<Uuid>` при клике

6. **`src/views/chat.rs`** — история сообщений
   - Разделители дат, пузырьки, статусы (⏳ ⌛ ✓ ✓✓)
   - Принимает `&EventSender` для кнопки удаления

7. **`src/views/compose.rs`** — поле ввода
   - Enter → `ComposeAction::Send(text)`, Shift+Enter → перенос
   - Кнопка отправить с анимацией цвета

8. **`src/views/contacts.rs`** — контакты
   - Список, форма добавления, статусы, кнопки «Написать» / удалить

9. **`src/app.rs`** — `EchatApp: impl eframe::App`
   - `update()`: poll_events → handle_event → draw_screen → draw_toasts
   - `spawn_login()` — через platform::build_app_state()
   - `select_conversation()` — загрузить историю, сбросить unread
   - `spawn_send_message()` / `spawn_add_contact()` / `spawn_delete_conversation()`

10. **`src/main.rs`** — `eframe::run_native()`, `tracing_subscriber::init()`
    - `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`

### Критерий готовности

```bash
cargo build -p echat-desktop
# Компилируется без ошибок
```

---

## Шаг 7 — `apps/mobile` (Rust FFI)

**Цель:** uniffi FFI-библиотека с polling API.

### Зависимости

```toml
[lib]
crate-type = ["cdylib", "staticlib"]

uniffi = { version = "0.28", features = ["build"] }

[build-dependencies]
uniffi = { version = "0.28", features = ["build"] }
```

### Порядок реализации

1. **`src/echat.udl`** — Interface Definition Language
   - Все dictionary типы: FfiAccount, FfiContact, FfiConversation, FfiMessage
   - `interface FfiEvent` с вариантами
   - `interface MobileApi` — все методы с `[Async]` где нужно
   - `[Error] enum MobileError`

2. **`build.rs`**
   ```rust
   uniffi::generate_scaffolding("src/echat.udl").unwrap();
   ```

3. **`src/types.rs`** — Rust-версии FFI структур + конвертации
   - `FfiAccount::from_account(acc, sync_connected)`
   - `FfiContact::from_contact(c)`
   - `FfiConversation::from_conversation(c, display_name)`
   - `FfiMessage::from_message(m, my_email)`
   - `impl From<core::Error> for MobileError`

4. **`src/events.rs`** — `EventQueue`
   - `Mutex<VecDeque<FfiEvent>>`, MAX_QUEUE_SIZE = 256
   - `push(event)` — из async потока
   - `drain()` — из UI потока
   - `subscribe_to_core_events(bus, queue, rt)` — tokio::spawn loop

5. **`src/api.rs`** — `MobileApi`
   - Поля: `rt`, `db_path`, `state: RwLock<Option<Arc<AppState>>>`, `account_id: RwLock<...>`, `my_email: RwLock<...>`, `events: EventQueue`
   - `login()` — build_app_state + add_account (или загрузить существующий)
   - `poll_events()` — drain + корректировать `is_mine`
   - Все async методы делегируют в `AppState` сервисы

6. **`src/lib.rs`** — `uniffi::include_scaffolding!("echat")`

### Критерий готовности

```bash
# Проверка что компилируется (без линковки под Android/iOS)
DATABASE_URL="sqlite::memory:" cargo check -p echat-mobile
```

---

## Шаг 8 — Android (Kotlin UI)

**Цель:** рабочее Android-приложение на Jetpack Compose.

### Требования

```
Android Studio Hedgehog+
Android NDK 25+
ANDROID_NDK_HOME установлен
```

### Шаги

1. Установить Rust таргеты и инструменты:
   ```bash
   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
   cargo install cargo-ndk uniffi-bindgen
   ```

2. Собрать `.so` и сгенерировать биндинги:
   ```bash
   cd apps/mobile && ./build-android.sh
   ```

3. Создать Android проект в `android/` (Jetpack Compose template)

4. **`EchatApplication.kt`** — инициализация в `onCreate()`

5. **`EchatViewModel.kt`** (уже реализован) — подключить к Activity

6. **`MainActivity.kt`** — `setContent { EchatApp(viewModel) }`

7. **`LoginScreen.kt`** (уже реализован)

8. **`ChatListScreen.kt`** (уже реализован)

9. **`ChatScreen.kt`** (уже реализован)

10. **`ContactsScreen.kt`** (уже реализован)

11. **`SyncForegroundService.kt`** — Foreground Service тип `dataSync`:
    ```kotlin
    class SyncForegroundService : Service() {
        override fun onStartCommand(...): Int {
            startForeground(NOTIF_ID, buildNotification())
            viewModel.onAppForeground()
            return START_STICKY
        }
    }
    ```

12. **Android Keystore JNI** (TODO):
    - Создать `KeystoreBridge.kt` с `EncryptedSharedPreferences`
    - Реализовать `platform/keystore/android.rs` через `jni` crate

### build.gradle (ключевые настройки)

```kotlin
android {
    defaultConfig {
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }
}

dependencies {
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.7.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.7.3")
}
```

---

## Шаг 9 — iOS (Swift UI)

**Цель:** рабочее iOS-приложение на SwiftUI.

### Требования

```
macOS 13+
Xcode 15+
Provisioning Profile (для тестирования на устройстве)
```

### Шаги

1. Установить таргеты:
   ```bash
   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
   cargo install uniffi-bindgen
   ```

2. Собрать XCFramework и Swift биндинги:
   ```bash
   cd apps/mobile && ./build-ios.sh
   ```

3. Создать iOS проект в `ios/` (SwiftUI template)

4. В Xcode: **File → Add Files → EchatMobile.xcframework**

5. Добавить сгенерированные Swift файлы из `ios/Sources/EChat/generated/`

6. **`EchatApp.swift`** — `@main` App entry point:
   ```swift
   @main
   struct EchatApp: App {
       var body: some Scene {
           WindowGroup { ContentView() }
       }
   }
   ```

7. **`ContentView.swift`** (уже реализован) — корневой навигатор

8. **`EchatViewModel.swift`** (уже реализован) — подключить `@StateObject`

9. **`LoginView.swift`**, **`ChatListView.swift`**, **`ChatView.swift`**, **`ContactsView.swift`** (уже реализованы)

10. **iOS Keychain bridge** (TODO):
    - `KeychainBridge.swift` → `SecItemAdd`, `SecItemCopyMatching`, `SecItemDelete`
    - Реализовать `platform/keystore/ios.rs` через `objc2` crate или uniffi callback

### Info.plist настройки

```xml
<!-- Фоновое выполнение для email синхронизации -->
<key>UIBackgroundModes</key>
<array>
    <string>fetch</string>
    <string>remote-notification</string>
</array>
```

---

## Шаг 10 — Финальная интеграция

**Цель:** все TODO закрыты, end-to-end тест на реальных устройствах.

### Чеклист

#### Критические (без них не работает)

- [ ] `core/sync/processor.rs::decrypt_message()` — реализовать через `StoragePort::get_contact_by_email()`
- [ ] `platform/keystore/android.rs` — JNI через `KeystoreBridge.kt`
- [ ] `platform/keystore/ios.rs` — Swift bridge через `KeychainBridge.swift`
- [ ] `SyncForegroundService.kt` — фоновая синхронизация Android

#### Важные (ухудшают UX)

- [ ] Infinite scroll в desktop — при скролле вверх загружать следующую страницу истории
- [ ] Индикатор отправки — обновлять `MessageStatus` после реального отправления
- [ ] Группы — ротация ключей при добавлении/удалении участника в `GroupService`

#### Опциональные

- [ ] Push-уведомления — FCM (Android) / APNs (iOS) через минимальный relay-сервер
- [ ] Поиск по истории — SQL LIKE по `body`
- [ ] Экспорт/импорт ключей через UI (уже есть в `encryption::export`)
- [ ] QR-код для обмена публичным ключом (qrcode + image крейты уже в workspace)
- [ ] Аватары контактов

### End-to-End тест

1. Установить на два устройства (или два аккаунта на одном)
2. Alice добавляет Bob → handshake
3. Убедиться что статус Bob стал `Active`
4. Alice отправляет сообщение
5. Bob получает, видит расшифрованный текст
6. Bob отвечает
7. Alice получает
8. Оба удаляют беседу — убедиться что письма удалены с IMAP сервера

---

## Справочник команд

```bash
# Запуск desktop
RUST_LOG=echat=debug cargo run -p echat-desktop

# Все тесты
DATABASE_URL="sqlite::memory:" cargo test \
    --workspace --exclude echat-desktop --exclude echat-mobile

# Конкретный крейт
cargo test -p encryption
DATABASE_URL="sqlite::memory:" cargo test -p storage
DATABASE_URL="sqlite::memory:" cargo test -p platform

# Live email тест
ECHAT_TEST_EMAIL=user@mail.ru ECHAT_TEST_PASS=password \
    cargo test -p email -- --ignored --nocapture

# Android сборка
cd apps/mobile && ./build-android.sh --release

# iOS сборка (macOS only)
cd apps/mobile && ./build-ios.sh --release

# Проверка без тестов
DATABASE_URL="sqlite::memory:" cargo check --workspace
```
