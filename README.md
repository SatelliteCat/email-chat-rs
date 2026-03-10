
# БД

Выполнение миграций

`$env:DATABASE_URL="sqlite://crates/storage/storage.db"; sqlx migrate run --source crates/storage/migrations`

`$env:DATABASE_URL="sqlite://crates/storage/storage.db"; cargo sqlx prepare --workspace`


Тестирование

`$env:DATABASE_URL="sqlite://crates/storage/storage.db"; cargo test -p storage`

или `export DATABASE_URL="sqlite::memory:"; cargo test -p storage`
