//! Экран настроек беседы.
//!
//! Содержит:
//! - Информацию о беседе
//! - Управление ключами шифрования
//! - Опасные действия (удаление беседы)

use egui::{
    Align, Color32, FontId, Frame, Layout, Margin, RichText, ScrollArea, Ui, Vec2,
    widgets::{Button, TextEdit},
};
use uuid::Uuid;

use crate::{
    runtime::{AppEvent, EventSender},
    state::{ChatUiState, ConversationItem, ConversationSettingsScreen},
    views::{sidebar::avatar_circle, theme},
};

/// Показывает экран настроек беседы.
pub fn show(
    ui: &mut Ui,
    settings: &mut ConversationSettingsScreen,
    chat: &mut ChatUiState,
    conv: &ConversationItem,
    sender: &EventSender,
) {
    // Заголовок экрана
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new("⚙️ Настройки беседы")
                .font(FontId::proportional(18.0))
                .color(theme::TEXT_PRIMARY)
                .strong(),
        );
    });
    ui.add_space(8.0);

    ScrollArea::vertical()
        .id_source("conv_settings_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            // Информация о беседе
            conversation_info(ui, conv);
            ui.add_space(16.0);

            // Ключи шифрования
            encryption_keys_section(ui, chat, conv.id, sender);
            ui.add_space(16.0);

            // Опасные действия
            danger_zone(ui, conv.id, sender);
            ui.add_space(16.0);

            // Кнопка "Назад"
            ui.horizontal(|ui| {
                if ui.button("← Назад к чату").clicked() {
                    settings.conv_id = None;
                }
            });
        });
}

/// Показывает информацию о беседе.
fn conversation_info(ui: &mut Ui, conv: &ConversationItem) {
    Frame::none()
        .fill(theme::BG_PANEL)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("💬 Информация о беседе")
                    .font(FontId::proportional(14.0))
                    .color(theme::TEXT_PRIMARY)
                    .strong(),
            );
            ui.add_space(8.0);

            // Аватар и название
            ui.horizontal(|ui| {
                avatar_circle(ui, conv.avatar_letter, 36.0, theme::ACCENT);

                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&conv.display_name)
                            .font(FontId::proportional(15.0))
                            .color(theme::TEXT_PRIMARY)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format!("ID: {}", conv.id))
                            .font(FontId::monospace(10.0))
                            .color(theme::TEXT_SECONDARY),
                    );
                });
            });

            ui.add_space(8.0);

            // Статистика
            if !conv.last_preview.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Последнее сообщение:")
                            .font(FontId::proportional(11.0))
                            .color(theme::TEXT_SECONDARY),
                    );
                    ui.label(
                        RichText::new(conv.last_preview.chars().take(50).collect::<String>())
                            .font(FontId::proportional(11.0))
                            .color(theme::TEXT_PRIMARY),
                    );
                });
            }

            if conv.unread > 0 {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Непрочитанных:")
                            .font(FontId::proportional(11.0))
                            .color(theme::TEXT_SECONDARY),
                    );
                    ui.label(
                        RichText::new(conv.unread.to_string())
                            .font(FontId::proportional(11.0))
                            .color(theme::ACCENT),
                    );
                });
            }

            if let Some(ref last_at) = conv.last_msg_at {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Последнее сообщение:")
                            .font(FontId::proportional(11.0))
                            .color(theme::TEXT_SECONDARY),
                    );
                    ui.label(
                        RichText::new(last_at.format("%d.%m.%Y %H:%M").to_string())
                            .font(FontId::proportional(11.0))
                            .color(theme::TEXT_PRIMARY),
                    );
                });
            }
        });
}

/// Секция управления ключами шифрования.
fn encryption_keys_section(ui: &mut Ui, chat: &mut ChatUiState, conv_id: Uuid, sender: &EventSender) {
    Frame::none()
        .fill(theme::BG_PANEL)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("🔑 Ключи шифрования")
                    .font(FontId::proportional(14.0))
                    .color(theme::TEXT_PRIMARY)
                    .strong(),
            );
            ui.add_space(6.0);

            ui.label(
                RichText::new("E2E шифрование использует X25519 + ChaCha20-Poly1305. Каждое сообщение шифруется уникальным общим секретом.")
                    .font(FontId::proportional(11.0))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(10.0);

            // Наш публичный ключ (для копирования)
            ui.label(
                RichText::new("Ваш публичный ключ (отправьте собеседнику):")
                    .font(FontId::proportional(11.0))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut chat.my_public_key)
                        .hint_text("Нажмите «Сгенерировать»")
                        .desired_width(f32::INFINITY)
                        .font(FontId::monospace(11.0)),
                );
                if ui.button("📋 Копировать").clicked() {
                    if !chat.my_public_key.is_empty() {
                        ui.ctx()
                            .output_mut(|o| o.copied_text = chat.my_public_key.clone());
                    }
                }
                if ui.button("🔄 Сгенерировать").clicked() {
                    sender.send(AppEvent::LoadConversationKeys { conv_id });
                }
            });

            ui.add_space(10.0);

            // Поле для вставки seed нашего ключа (для восстановления истории)
            ui.label(
                RichText::new("Ваш seed ключа (для расшифровки старых сообщений):")
                    .font(FontId::proportional(11.0))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut chat.my_keypair_seed_input)
                        .hint_text("Вставьте base64 seed вашего ключа здесь...")
                        .desired_width(f32::INFINITY)
                        .font(FontId::monospace(11.0)),
                );
                if ui.button("📥 Импортировать").clicked() {
                    if !chat.my_keypair_seed_input.trim().is_empty() {
                        sender.send(AppEvent::ImportMyKeypairSeed {
                            conv_id,
                            seed_base64: chat.my_keypair_seed_input.clone(),
                        });
                    }
                }
            });

            ui.add_space(10.0);

            // Поле для вставки публичного ключа собеседника
            ui.label(
                RichText::new("Публичный ключ собеседника:")
                    .font(FontId::proportional(11.0))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut chat.their_public_key_input)
                        .hint_text("Вставьте публичный ключ собеседника здесь...")
                        .desired_width(f32::INFINITY)
                        .font(FontId::monospace(11.0)),
                );
                if ui.button("✓ Сохранить").clicked() {
                    if !chat.their_public_key_input.trim().is_empty() {
                        sender.send(AppEvent::SetTheirPublicKey {
                            conv_id,
                            public_key_json: chat.their_public_key_input.clone(),
                        });
                    }
                }
            });

            // Статус
            if let Some(ref status) = chat.keys_status_message {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(status)
                        .font(FontId::proportional(10.5))
                        .color(theme::TEXT_TIMESTAMP),
                );
            }
        });
}

/// Секция опасных действий.
fn danger_zone(ui: &mut Ui, conv_id: Uuid, sender: &EventSender) {
    Frame::none()
        .fill(Color32::from_rgb(40, 20, 20))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(120, 40, 40)))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("⚠️ Опасные действия")
                    .font(FontId::proportional(14.0))
                    .color(Color32::from_rgb(220, 80, 80))
                    .strong(),
            );
            ui.add_space(6.0);

            ui.label(
                RichText::new("Удаление беседы удалит все сообщения из базы данных и с почтового сервера. Это действие необратимо.")
                    .font(FontId::proportional(11.0))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), 32.0));
                if ui
                    .add(
                        Button::new("🗑️ Удалить беседу")
                            .fill(Color32::from_rgb(180, 50, 50))
                            .stroke(egui::Stroke::NONE)
                            .rounding(egui::Rounding::same(6.0)),
                    )
                    .clicked()
                {
                    sender.send(AppEvent::DeleteConversation { conv_id });
                }
            });
        });
}
