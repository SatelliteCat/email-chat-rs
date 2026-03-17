//! Экран управления контактами.

use egui::{Color32, FontId, Frame, Margin, RichText, Rounding, Ui, Vec2};

use echat_core::models::contact::{Contact, ContactStatus};

use crate::{runtime::AppEvent, runtime::EventSender, state::UiState, views::theme};

/// Рисует экран контактов.
pub fn show(ui: &mut Ui, state: &mut UiState, sender: &EventSender) {
    Frame::none()
        .fill(theme::BG_DARK)
        .inner_margin(Margin::same(20.0))
        .show(ui, |ui| {
            // ── Шапка ──────────────────────────────────────────────────────
            ui.horizontal(|ui| {
                if ui.button("← Назад").clicked() {
                    state.screen = crate::state::Screen::Main;
                }
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Контакты")
                        .font(FontId::proportional(18.0))
                        .color(theme::TEXT_PRIMARY)
                        .strong(),
                );
            });

            ui.add_space(16.0);

            // ── Форма добавления контакта ───────────────────────────────────
            Frame::none()
                .fill(theme::BG_PANEL)
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Добавить контакт")
                            .color(theme::TEXT_SECONDARY)
                            .font(FontId::proportional(13.0)),
                    );
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.new_contact_email)
                                .hint_text("Email")
                                .desired_width(200.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut state.new_contact_name)
                                .hint_text("Имя (необязательно)")
                                .desired_width(160.0),
                        );

                        let can_add = !state.new_contact_email.trim().is_empty();
                        if ui
                            .add_enabled(
                                can_add,
                                egui::Button::new(RichText::new("Добавить").color(Color32::WHITE))
                                    .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            let email = state.new_contact_email.trim().to_lowercase();
                            let name = if state.new_contact_name.trim().is_empty() {
                                email.clone()
                            } else {
                                state.new_contact_name.trim().to_string()
                            };
                            state.new_contact_email.clear();
                            state.new_contact_name.clear();
                            sender.send(AppEvent::AddContact { email, name });
                        }
                    });
                });

            ui.add_space(16.0);

            // ── Поиск ──────────────────────────────────────────────────────
            ui.add(
                egui::TextEdit::singleline(&mut state.contact_search)
                    .hint_text("🔍 Поиск контактов")
                    .desired_width(f32::INFINITY),
            );

            ui.add_space(12.0);

            // ── Список контактов ───────────────────────────────────────────
            let search = state.contact_search.to_lowercase();

            egui::ScrollArea::vertical().show(ui, |ui| {
                let contacts: Vec<_> = state
                    .contacts
                    .iter()
                    .filter(|c| {
                        search.is_empty()
                            || c.name.to_lowercase().contains(&search)
                            || c.email.to_lowercase().contains(&search)
                    })
                    .cloned()
                    .collect();

                if contacts.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Нет контактов").color(theme::TEXT_TIMESTAMP));
                    });
                }

                for contact in &contacts {
                    contact_row(ui, contact, sender);
                    ui.add_space(2.0);
                }
            });
        });
}

fn contact_row(ui: &mut Ui, contact: &Contact, sender: &EventSender) {
    Frame::none()
        .fill(theme::BG_PANEL)
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                // Аватар
                let letter = contact
                    .name
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .next()
                    .unwrap_or('?');
                super::sidebar::avatar_circle(ui, letter, 36.0, theme::ACCENT);
                ui.add_space(10.0);

                // Имя и email
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&contact.name)
                            .color(theme::TEXT_PRIMARY)
                            .font(FontId::proportional(14.0))
                            .strong(),
                    );
                    ui.label(
                        RichText::new(&contact.email)
                            .font(FontId::proportional(12.0))
                            .color(theme::TEXT_SECONDARY),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Статус — показывает наличие публичного ключа
                    let (status_text, status_color) = match contact.status {
                        ContactStatus::HasKey => ("● ключ есть", theme::SUCCESS),
                        ContactStatus::NoKey => ("● нет ключа", theme::TEXT_TIMESTAMP),
                    };
                    ui.label(
                        RichText::new(status_text)
                            .font(FontId::proportional(11.5))
                            .color(status_color),
                    );

                    ui.add_space(8.0);

                    // Кнопка «Написать» — доступна для контактов с любым статусом
                    if ui
                        .add(
                            egui::Button::new(RichText::new("💬").font(FontId::proportional(14.0)))
                                .frame(false),
                        )
                        .on_hover_text("Открыть чат")
                        .clicked()
                    {
                        sender.send(AppEvent::OpenChatWith {
                            contact_id: contact.id,
                        });
                    }

                    ui.add_space(8.0);

                    // Кнопка «Удалить»
                    if ui
                        .add(
                            egui::Button::new(RichText::new("🗑").font(FontId::proportional(14.0)))
                                .frame(false)
                                .fill(egui::Color32::from_rgb(60, 20, 20)),
                        )
                        .on_hover_text("Удалить контакт")
                        .clicked()
                    {
                        sender.send(AppEvent::DeleteContact {
                            contact_id: contact.id,
                        });
                    }
                });
            });
        });
}
