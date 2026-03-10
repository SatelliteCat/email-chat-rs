//! Trait-интерфейсы (Ports) — изолируют ядро от конкретных реализаций.
//!
//! Ядро знает только об этих трейтах. Конкретные реализации
//! (`crates/email`, `crates/storage`, `crates/platform`) подключаются
//! снаружи через dependency injection в `AppState`.

pub mod email;
pub mod keystore;
pub mod storage;
