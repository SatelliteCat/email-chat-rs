//! # EChat Desktop
//!
//! Точка входа. Инициализирует логирование и запускает eframe.
//!
//! ## Архитектура потоков
//!
//! ```text
//! main thread  →  eframe (egui render loop, ~60fps)
//!                    │
//!                    └─► AsyncRuntime  →  tokio multi-thread pool
//!                                            ├─ IMAP IDLE задача
//!                                            ├─ send_message задачи
//!                                            └─ load_history задачи
//! ```

// Скрываем консоль на Windows в release-сборке
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod runtime;
mod state;
mod views;

use app::EchatApp;

fn main() -> eframe::Result<()> {
    // Логирование: RUST_LOG=echat=debug cargo run
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("echat=info,warn")),
        )
        .with_target(false)
        .compact()
        .init();

    tracing::info!("EChat Desktop v{}", env!("CARGO_PKG_VERSION"));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("EChat")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([640.0, 480.0])
            .with_icon(load_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "EChat",
        native_options,
        Box::new(|cc| Ok(Box::new(EchatApp::new(cc)))),
    )
}

/// Загружает иконку приложения из ресурсов (или возвращает пустую).
fn load_icon() -> egui::IconData {
    // В реальном проекте: include_bytes!("../assets/icon.png") + decode
    // Пока возвращаем пустую — eframe использует иконку ОС по умолчанию
    egui::IconData::default()
}
