
# БД

## Выполнение миграций

### win

`$env:DATABASE_URL="sqlite://crates/storage/storage.db"; sqlx migrate run --source crates/storage/migrations`

`$env:DATABASE_URL="sqlite://crates/storage/storage.db"; cargo sqlx prepare --workspace`

### linux

При первом запуске `env DATABASE_URL="sqlite://crates/storage/storage.db" sqlx database setup --source crates/storage/migrations`

`DATABASE_URL="sqlite://crates/storage/storage.db" sqlx migrate run --source crates/storage/storage.db`

`DATABASE_URL="sqlite://crates/storage/storage.db" cargo sqlx prepare --workspace`


Тестирование

`$env:DATABASE_URL="sqlite://crates/storage/storage.db"; cargo test -p storage`

или `export DATABASE_URL="sqlite::memory:"; cargo test -p storage`


# Запуск

## Debug (с консолью)

`cargo run -p echat-desktop`

## Release (без консоли на Windows)

`cargo build -p echat-desktop --release`

## Crosscompiling

`cargo build --target x86_64-pc-windows-gnu --release -p echat-desktop`

### Зависимости

`rustup target add x86_64-pc-windows-gnu`

`sudo pacman -S mingw-w64-gcc`
