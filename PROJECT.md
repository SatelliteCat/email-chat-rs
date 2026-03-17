# EChat — Проектная документация

> Зашифрованный мессенджер, использующий e-mail как транспорт.  
> Полный стек на Rust. Платформы: Windows, Linux, Android, iOS.  
> Провайдеры: mail.ru, yandex.ru.

---

## Содержание

1. [Концепция](#концепция)
2. [Архитектура](#архитектура)
3. [Структура workspace](#структура-workspace)
4. [Шифрование](#шифрование)
5. [Email-транспорт](#email-транспорт)
6. [База данных](#база-данных)
7. [Ядро (core)](#ядро-core)
8. [Platform-адаптеры](#platform-адаптеры)
9. [Desktop-приложение](#desktop-приложение)
10. [Мобильное приложение](#мобильное-приложение)
11. [Ключевые решения](#ключевые-решения)
12. [Известные TODO](#известные-todo)
13. [Сборка и запуск](#сборка-и-запуск)

---

## Концепция

Приложение маскируется под обычную почтовую переписку. Обычный почтовый клиент видит:

```
От:   alice@mail.ru
Кому: bob@yandex.ru
Тема: Re: документы     ← случайная из 16 предустановленных тем
X-EChat: 1              ← единственный кастомный заголовок

[base64-блоб, нечитаемый без ключа]
```

Magic bytes `[0xEC, 0xC4, 0xA7, 0x01]` в начале каждого payload — быстрое обнаружение без расшифровки. Если `X-EChat` отсутствует — fallback к проверке magic bytes.

---

## Архитектура

```
┌──────────────────────────────────────────────────────────┐
│           apps/desktop (egui)  /  apps/mobile (uniffi)   │
└────────────────────────┬─────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────┐
│                   crates/platform                        │
│      EmailAdapter   StorageAdapter   PlatformKeystore    │
│                   build_app_state()                      │
└───────┬───────────────────┬────────────────────┬─────────┘
        │                   │                    │
┌───────▼───────┐  ┌────────▼──────┐  ┌──────────▼───────┐
│ crates/email  │  │crates/storage │  │   OS Keychain     │
│  IMAP / SMTP  │  │ SQLite + sqlx │  │ Win/mac/Linux/    │
└───────────────┘  └───────────────┘  │ Android / iOS     │
        ▲                   ▲         └──────────────────-─┘
        │   реализуют       │
┌───────┴───────────────────┴──────────────────────────────┐
│                   crates/core                            │
│  ports/EmailTransport  StoragePort  KeystorePort         │
│  models/Account  Contact  Conversation  Message          │
│  services/AccountService  ContactService                 │
│           ChatService  GroupService                      │
│  SyncEngine (IMAP IDLE loop)   EventBus                  │
└──────────────────────────────────────────────────────────┘
                         ▲
                         │ использует
┌────────────────────────┴─────────────────────────────────┐
│                 crates/encryption                        │
│  IdentityKeypair   SharedSecret    EncryptedPayload      │
│  HandshakeMessage  GroupEncryptedPayload  ExportedKey    │
└──────────────────────────────────────────────────────────┘
```

**Принцип зависимостей:** `core` зависит только от `encryption`. Реализации `email` и `storage` инжектируются через трейты — `core` про них ничего не знает.

---

## Структура workspace

```
email-chat/
├── Cargo.toml                   # workspace root, все версии зависимостей
├── PROJECT.md                   # этот файл
├── IMPLEMENTATION.md            # шаги реализации
│
├── crates/
│   ├── encryption/              # крипто-примитивы (X25519, ChaCha20, Ed25519)
│   ├── email/                   # IMAP/SMTP клиент (mail.ru, yandex.ru)
│   ├── storage/                 # SQLite через sqlx, 4 миграции, 4 репозитория
│   ├── core/                    # бизнес-логика, трейты, сервисы, SyncEngine
│   └── platform/                # адаптеры, OS keystore, build_app_state()
│
└── apps/
    ├── desktop/                 # egui UI (Windows, Linux, macOS)
    └── mobile/
        ├── src/                 # Rust FFI через uniffi
        ├── android/             # Kotlin + Jetpack Compose
        ├── ios/                 # Swift + SwiftUI
        ├── build-android.sh     # скрипт сборки Android
        └── build-ios.sh         # скрипт сборки iOS
```

**Порядок зависимостей:**
```
desktop/mobile → platform → core ← email, storage, encryption
```

---

## Шифрование

### Алгоритмы

| Назначение | Алгоритм | Крейт |
|---|---|---|
| Обмен ключами | X25519 (ECDH) | `x25519-dalek` |
| Подписи | Ed25519 | `ed25519-dalek` |
| Симметричное шифрование | ChaCha20-Poly1305 AEAD | `chacha20poly1305` |
| Деривация ключа | HKDF-SHA256 | `hkdf` + `sha2` |
| Пароль на экспорт | Argon2id | `argon2` |
| Мнемоника | BIP-39 (24 слова) | `bip39` |

### IdentityKeypair

Из единого 32-байтного seed выводятся два ключа:
- X25519 (для DH) — напрямую из seed
- Ed25519 (для подписей) — через `SHA-256("ed25519-derive-v1" || seed)`

Разделение предотвращает случайное смешение контекстов. Seed хранится в OS keystore, никогда в БД.

### Прямое сообщение

```
Shared = X25519(my_secret, their_public)
Key    = HKDF-SHA256(Shared, info="direct-chat")
Body   = ChaCha20-Poly1305(plaintext, Key, random_nonce_12bytes)
```

Нулевой DH-результат отклоняется (защита от small subgroup attack).

### Групповое сообщение (fan-out)

```
session_key = OsRng 32 bytes
body        = ChaCha20-Poly1305(plaintext, session_key)

for each member:
    shared_i    = X25519(sender_secret, member_public)
    key_i       = HKDF(shared_i, info="group-{group_uuid}")
    wrapped_key = ChaCha20-Poly1305(session_key, key_i)

payload = { nonce, ciphertext, member_keys: { email → wrapped_key } }
```

`session_key` обнуляется через `zeroize` после использования.

### Handshake

```
Alice                                Bob
  │── HandshakeInit ───────────────► │
  │   X25519_pub, Ed25519_pub,        │
  │   Ed25519_sign(keys || ts), ts    │
  │                                   │── создаёт Contact(Alice)
  │ ◄── HandshakeAck ─────────────── │
  │     Bob's keys + signature        │
```

Подпись покрывает: `x25519_pub || ed25519_pub || timestamp_le_bytes`.  
Защита от replay: проверка timestamp (по умолчанию max_age = 24 ч).

### Ротация ключей группы

- **Добавление:** новый `session_key`, GroupKeyUpdate всем включая нового. Новый участник не может читать историю.
- **Удаление:** ротация `session_key`, GroupKeyUpdate только оставшимся.

### Экспорт / резервная копия

Только ручной экспорт — пользователь сам отвечает за сохранность:
- **BIP-39:** 24 слова, можно записать от руки
- **Base64 файл:** Argon2id-зашифрован паролем (64MB RAM, 3 итерации, 4 параллелизма — OWASP 2024)

Checksum: `hex(SHA-256(seed)[0..4])` — быстрая проверка неверного пароля без полной дешифровки.

### Контакт без приложения

```
[текстовое объяснение + ссылка на скачивание]
---
[base64-payload]
```

Сообщение ставится в очередь (`status = Queued`). После получения Ack — автоматически отправляется.

---

## Email-транспорт

### Провайдеры

| Домен | IMAP | SMTP |
|---|---|---|
| mail.ru, inbox.ru, list.ru, bk.ru | imap.mail.ru:993 | smtp.mail.ru:465 |
| yandex.ru, ya.ru, yandex.com, yandex.kz, yandex.by, yandex.ua | imap.yandex.ru:993 | smtp.yandex.ru:465 |

TLS: только implicit (порт 993/465). STARTTLS не используется.  
Используется **пароль приложения** — не основной пароль аккаунта.

### ChatEnvelope (внутренняя структура)

```json
{
  "msg_id":          "uuid",
  "conv_id":         "uuid",
  "kind":            "Message | Handshake | HandshakeAck | Invite",
  "sent_at":         "ISO8601",
  "body":            "base64(encrypted-payload)",
  "reply_to":        "uuid | null",
  "protocol_version": 1
}
```

Весь envelope шифруется целиком. Снаружи видны только email-заголовки.

### Особенности реализации

- **IMAP IDLE таймаут** — 29 минут (RFC: не более 30). После — автореконнект.
- **Папка EChat** — все письма хранятся в отдельной папке `"EChat"`, создаётся автоматически.
- **Удаление** — `STORE +FLAGS \Deleted` + `EXPUNGE`. Полное удаление, не в Trash.
- **native-tls** вместо rustls — системные сертификаты на мобильных платформах.
- **lettre** для SMTP — новый `AsyncSmtpTransport` для каждого письма (нет постоянного соединения).

---

## База данных

### Путь к файлу

| Платформа | Путь |
|---|---|
| Linux | `~/.local/share/echat/db.sqlite` |
| Windows | `%LOCALAPPDATA%\echat\db.sqlite` |
| macOS | `~/Library/Application Support/echat/db.sqlite` |
| Android | `<filesDir>/echat.sqlite` |
| iOS | `<Documents>/echat.sqlite` |

### Настройки при открытии

```sql
PRAGMA journal_mode = WAL;        -- параллельное чтение
PRAGMA synchronous   = NORMAL;    -- баланс надёжности и скорости
PRAGMA foreign_keys  = ON;        -- каскадные удаления
```

Миграции запускаются автоматически через `sqlx::migrate!()` при каждом `Database::open()`.

### Схема

```sql
accounts (id UUID, email, provider, imap_host, imap_port, smtp_host, smtp_port,
          echat_folder, last_imap_uid, last_sync_at, is_active)

contacts (id UUID, account_id FK→accounts, name, email, avatar BLOB, status,
          public_keys_json, handshake_at)
  UNIQUE(account_id, email)

conversations (id UUID, account_id FK→accounts, kind, contact_id,
               group_name, group_avatar,
               last_msg_at, last_msg_preview, unread_count)

group_members (conversation_id FK, contact_id FK, role, joined_at,
               public_key_snapshot)

messages (id UUID, conversation_id FK, account_id FK, from_email, body,
          kind, status, reply_to UUID, imap_uid, imap_folder, sent_at, received_at)
  INDEX: (conversation_id, sent_at DESC)   -- история / пагинация
  INDEX: (conversation_id, imap_uid)       -- удаление с сервера
  INDEX: (from_email)                      -- дедупликация
```

### Хранение ключей

- **Публичные ключи контактов** → `contacts.public_keys_json` (JSON: `{x25519, ed25519}`)
- **Приватный ключ (seed)** → **никогда в БД**, только через `KeystorePort`

### Compile-time проверка sqlx

`query!` макросы проверяют SQL при сборке. Требуют:
```bash
export DATABASE_URL="sqlite::memory:"
cargo build -p storage
```

---

## Ядро (core)

### Порты (трейты-интерфейсы)

| Трейт | Назначение |
|---|---|
| `EmailTransport` | send, fetch_new, idle_wait, delete_messages, ensure_echat_folder |
| `StoragePort` | полный CRUD: accounts, contacts, conversations, messages |
| `KeystorePort` | set(service, key, bytes), get, delete |

Константы keystore:
```
SERVICE_MAIL     = "mail"      ключ: "app_password:{email}"
SERVICE_IDENTITY = "identity"  ключ: "seed:{account_id}"
```

### Сервисы

| Сервис | Ответственность |
|---|---|
| `AccountService` | Добавление аккаунта, сохранение credentials в keystore, загрузка keypair |
| `ContactService` | Добавление контакта, инициация/обработка handshake, CRUD |
| `ChatService` | Отправка/получение сообщений, cursor pagination истории, удаление беседы (EXPUNGE + DB) |
| `GroupService` | Создание группы, добавление/удаление участников, fan-out отправка |

### SyncEngine

```
tokio::spawn → IMAP IDLE loop
    │
    ├─ idle_wait() (29 мин или push-уведомление)
    ├─ fetch_new_messages(since_uid)
    ├─ process_incoming() → HandshakeInit/Ack/EncryptedMessage
    │
    ├─ reconnect экспоненциальный: 5с → 10с → 20с → ... → 5 мин
    │
    └─ SyncCommand канал: FetchNow | Stop
```

### EventBus

`tokio::broadcast::Sender<ChatEvent>` — события публикуются из сервисов, подписчики независимы:

```
ChatEvent::NewMessage { conversation_id, message }
ChatEvent::ContactActivated { contact_id, email }
ChatEvent::SyncStateChanged { connected }
ChatEvent::SyncError { message }
ChatEvent::GroupMemberAdded / GroupMemberRemoved
```

### Жизненный цикл контакта

```
Unregistered  ──(handshake sent)──►  Pending  ──(ack received)──►  Active
```

### Жизненный цикл сообщения

```
Queued  ──►  Sending  ──►  Sent  ──►  Delivered  ──►  Read
```

`Queued` = сообщение ожидает handshake (контакт не установил приложение).

### Cursor pagination истории

```sql
WHERE conversation_id = ? AND sent_at < :before
ORDER BY sent_at DESC
LIMIT :limit
```

Следующая страница: передать `sent_at` самого раннего сообщения из предыдущей.

---

## Platform-адаптеры

### Конвертация типов (StorageAdapter)

Конвертация `core::models ↔ storage::models` намеренно **явная** (не через `From`/`Into`). Цель: изменения схемы БД не ломают интерфейс core незаметно — компилятор укажет на все места.

### Desktop Keystore (keyring crate)

Хранит: `(service="echat.{service}", username={key}) → base64(bytes)`.

Бинарные данные → Base64, т.к. keyring хранит строки. Все вызовы через `spawn_blocking` (API синхронный).

Ошибка `keyring::Error::NoEntry` при `get()` и `delete()` — не ошибка, возвращает `None`/`Ok(())`.

### Android Keystore (stub)

Требует JNI-моста:
```
Rust → JNI → KeystoreBridge.kt → EncryptedSharedPreferences → Android Keystore
```

### iOS Keychain (stub)

Требует Swift/objc2 моста:
```
Rust → objc2 / uniffi → KeychainBridge.swift → SecItemAdd/Get/Delete
```

### build_app_state()

Единственная точка сборки для desktop и mobile:
```
1. Database::open(db_path)   → SQLite + миграции
2. ProviderConfig::detect()  → определить провайдера по домену
3. EmailClient::connect()    → IMAP LOGIN
4. PlatformKeystore::new()
5. AppState::new(email, storage, keystore, config)
```

---

## Desktop-приложение

### Фреймворк: egui + eframe 0.29

Immediate mode: всё перерисовывается каждый кадр (~60fps). Нет state-менеджмента компонентов, нет Virtual DOM.

### Async ↔ egui мост

```
egui update() [sync, UI thread]
    │
    ├─► rt.spawn(async { ... })    →  tokio multi-thread pool
    │       после завершения:
    │       event_tx.send(AppEvent)
    │       ctx.request_repaint()  ←── будит egui из другого потока
    │
    └─► event_rx.try_recv()        ←  считываем все накопившиеся события
               │
               └─► handle_event() → UiState обновляется
```

### Экраны

| Экран | Файл |
|---|---|
| Логин | `views/login.rs` — Email + пароль приложения, валидация, spinner |
| Список бесед (sidebar) | `views/sidebar.rs` — аватары, непрочитанные, поиск, статус sync |
| История сообщений | `views/chat.rs` — пузырьки, разделители дат, статусы (⏳✓✓✓) |
| Поле ввода | `views/compose.rs` — Enter отправляет, Shift+Enter — перенос строки |
| Контакты | `views/contacts.rs` — добавить, статус, кнопка «Написать» |

### Тёмная тема

Константы в `views/mod.rs::theme`, применяются один раз при старте:

```
BG_DARK      #12121A    BG_PANEL     #1A1A22    BG_HOVER    #26263A
BG_MSG_OUT   #254090    BG_MSG_IN    #28283A
ACCENT       #508CFF    SUCCESS      #50C878    ERROR       #DC5050
TEXT_PRIMARY #DCDCE6    TEXT_SECONDARY #8C8CA0  TEXT_TIMESTAMP #64648C
```

### AppEvent (полный список)

```rust
LoginAttempt { email, password }   AccountReady { state, account }
AccountError(String)
ConversationsLoaded(Vec<Conversation>)   ContactsLoaded(Vec<Contact>)
HistoryLoaded { conv_id, messages }
NewMessage { conv_id, message }    MessageSent { conv_id, message }
SendError(String)
AddContact { email, name }         ContactAdded   ContactError(String)
OpenChatWith { contact_id }
DeleteConversation { conv_id }     ConversationDeleted { conv_id }
SyncConnected(bool)                SyncError(String)
```

### Удаление беседы

`AppEvent::DeleteConversation` → `chat_service.delete_conversation(id, true)` → IMAP EXPUNGE + SQLite CASCADE → `AppEvent::ConversationDeleted` → убрать из UI, сбросить selected.

---

## Мобильное приложение

### Rust FFI (uniffi)

`echat.udl` — единственный источник правды. uniffi генерирует:
- `EchatMobile.kt` для Android
- `EchatMobile.swift` для iOS

**FFI-типы** — только примитивы (`String`, `i64`, `bool`, `u32`). Никаких `Uuid`, `DateTime`, Rust-перечислений. Timestamps — Unix секунды в `i64`.

### MobileApi

```
new(db_path, app_download_url)    → Result<MobileApi>   [sync]
login(email, password)            → async Result<FfiAccount>
current_account()                 → Option<FfiAccount>  [sync]
start_sync() / stop_sync()        [sync]
shutdown()                        [sync]

list_conversations()              → async Result<Vec<FfiConversation>>
get_history(conv_id, before?, limit) → async Result<Vec<FfiMessage>>
mark_read(conv_id)                → async Result<()>
delete_conversation(conv_id)      → async Result<()>

send_message(conv_id, contact_id, text) → async Result<FfiMessage>

list_contacts()                   → async Result<Vec<FfiContact>>
add_contact(email, name)          → async Result<FfiContact>
delete_contact(contact_id)        → async Result<()>

poll_events()                     → Vec<FfiEvent>        [sync, non-blocking]
```

### EventQueue (polling архитектура)

Причина: callbacks через uniffi требуют сложной синхронизации между Rust-мьютексами и JVM/ObjC рантаймом. Polling проще и достаточен для чата.

```
SyncEngine (tokio) → ChatEvent
                         ↓
                  subscribe_to_core_events()
                         ↓
              EventQueue::push()   [async thread]
              Mutex<VecDeque<FfiEvent>>
                         ↓
              poll_events() → drain()  [UI thread, каждые 200мс]
```

Защита от переполнения: при 256 событиях старые отбрасываются с предупреждением.

`is_mine` в `FfiMessage` корректируется в `poll_events()` — только там известен `my_email`.

### Android (Kotlin)

**Паттерн ViewModel:**
```kotlin
viewModelScope.launch(Dispatchers.IO) {
    val result = api.someMethod()   // Rust, IO поток
    _state.update { it.copy(...) } // StateFlow → Compose
}
```

**Polling:**
```kotlin
viewModelScope.launch {
    while (isActive) {
        delay(200)
        processEvents()  // api.pollEvents() без блокировки
    }
}
```

**Foreground Service** (`SyncForegroundService`, тип `dataSync`): без него Android убивает IMAP IDLE через ~5 минут в фоне.

**Lifecycle:**
```kotlin
onAppBackground() → api.stopSync()
onAppForeground() → api.startSync()
onCleared()       → pollJob.cancel(); api.shutdown()
```

### iOS (Swift)

**Паттерн ObservableObject:**
```swift
@MainActor
class EchatViewModel: ObservableObject {
    @Published var conversations: [FfiConversation] = []
}
```

Все вызовы через `Task { await api.method() }` — Swift concurrency автоматически держит UI на MainActor.

**Polling:**
```swift
Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { _ in
    Task { @MainActor in self.processEvents() }
}
```

**Lifecycle:**
```swift
.onReceive(UIApplication.willResignActiveNotification) { vm.onBackground() }
.onReceive(UIApplication.didBecomeActiveNotification)  { vm.onForeground() }
```

---

## Ключевые решения

### native-tls вместо rustls для email

`rustls` требует ручной настройки корневых сертификатов на Android и iOS. `native-tls` использует системное хранилище — работает из коробки на всех платформах.

### Polling вместо FFI callbacks

uniffi поддерживает callbacks через `[Trait]`, но они требуют сложной синхронизации между Rust-мьютексами и JVM/ObjC рантаймом — риск deadlock. Polling с интервалом 200мс проще, надёжнее и незаметен для пользователя.

### Явная конвертация типов в StorageAdapter

Конвертация `core ↔ storage` не использует `From`/`Into` — намеренно verbose. Цель: любое изменение схемы БД немедленно вызывает ошибку компиляции во всех местах, а не тихий баг в рантайме.

### Один seed для двух ключей

Из одного 32-байтного seed выводятся X25519 и Ed25519. Пользователю нужно хранить только одну резервную копию. Ed25519 выводится через SHA-256 с доменным разделителем — нет риска смешения контекстов.

### Папка EChat в IMAP

Все echat-письма в отдельной папке, не в Inbox. Преимущества: меньше мусора, проще синхронизация (IDLE только на одной папке), очевидно пользователю.

### egui для desktop

Один .exe без Node.js и web-рантайма. Immediate mode хорош для чата — список сообщений с частыми обновлениями. Всё на Rust — единый инструментарий.

---

## Известные TODO

| Место | Описание |
|---|---|
| `core/sync/processor.rs::decrypt_message()` | Помечен TODO — нужен `StoragePort` для поиска публичного ключа контакта. Будет завершён при интеграции с platform. |
| `platform/keystore/android.rs` | Stub — требует JNI + `KeystoreBridge.kt` + EncryptedSharedPreferences |
| `platform/keystore/ios.rs` | Stub — требует `objc2` или Swift bridge + `KeychainBridge.swift` |
| `apps/mobile` SyncForegroundService | `SyncForegroundService.kt` для фонового IMAP IDLE на Android |
| Ротация ключей группы | Описана в документации, не реализована в `GroupService` |
| Infinite scroll в desktop | Загружается первые 60 сообщений, пагинация при скролле вверх не реализована |
| Уведомления | Push-уведомления на мобильных (FCM/APNs через свой сервер) не реализованы |

---

## Сборка и запуск

### Требования

```
Rust 1.75+  (edition 2021)
```

### Desktop

```bash
# Debug с логами
RUST_LOG=echat=debug cargo run -p echat-desktop

# Release
cargo build -p echat-desktop --release
# → target/release/echat  (Linux/macOS)
# → target/release/echat.exe  (Windows, без консоли)
```

### Тесты

```bash
# Шифрование — без внешних зависимостей
cargo test -p encryption

# Email — unit тесты
cargo test -p email

# Email — live тесты с реальным сервером
ECHAT_TEST_EMAIL=user@mail.ru ECHAT_TEST_PASS=app_password \
    cargo test -p email -- --ignored --nocapture

# Storage и Platform — требуют DATABASE_URL
DATABASE_URL="sqlite::memory:" cargo test -p storage
DATABASE_URL="sqlite::memory:" cargo test -p platform

# Core
cargo test -p core

# Все (кроме desktop/mobile и live email)
DATABASE_URL="sqlite::memory:" cargo test \
    --workspace --exclude echat-desktop --exclude echat-mobile
```

### Android

```bash
# Установить таргеты и инструменты
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk uniffi-bindgen

# Собрать всё одной командой
cd apps/mobile
./build-android.sh --release

# Открыть android/ в Android Studio → Sync → Run
```

### iOS (только macOS + Xcode)

```bash
# Установить таргеты
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install uniffi-bindgen

# Собрать всё одной командой
cd apps/mobile
./build-ios.sh --release

# Открыть ios/ в Xcode → Add EchatMobile.xcframework → Run
```
