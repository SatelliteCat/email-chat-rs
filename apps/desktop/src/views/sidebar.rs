//! Сайдбар — левая панель со списком бесед.

use chrono::{Local, TimeZone};
use egui::{
    Align, Align2, Color32, FontId, Frame, Label, Layout, Margin, Response, RichText, Rounding,
    Sense, Stroke, Ui, Vec2,
};
use uuid::Uuid;

use crate::{
    state::{ConversationItem, UiState},
    views::theme,
};

/// Рисует левую панель. Возвращает ID беседы если пользователь кликнул на неё.
pub fn show(ui: &mut Ui, state: &mut UiState) -> Option<Uuid> {
    let mut selected = None;

    Frame::none().fill(theme::BG_PANEL).show(ui, |ui| {
        ui.set_min_width(270.0);
        ui.set_max_width(320.0);

        // ── Шапка ──────────────────────────────────────────────────────
        Frame::none()
            .fill(theme::BG_DARK)
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Аватар пользователя (заглушка)
                    let letter = state
                        .account
                        .as_ref()
                        .and_then(|a| a.email.chars().next())
                        .unwrap_or('?')
                        .to_uppercase()
                        .next()
                        .unwrap_or('?');

                    avatar_circle(ui, letter, 32.0, theme::ACCENT);

                    ui.add_space(8.0);

                    ui.vertical(|ui| {
                        let email = state
                            .account
                            .as_ref()
                            .map(|a| a.email.as_str())
                            .unwrap_or("—");
                        ui.label(
                            RichText::new(email)
                                .color(theme::TEXT_PRIMARY)
                                .font(FontId::proportional(13.5))
                                .strong(),
                        );
                        // Индикатор подключения
                        let (dot, color, label) = if state.sync_connected {
                            ("●", theme::SUCCESS, "онлайн")
                        } else {
                            ("●", theme::TEXT_TIMESTAMP, "оффлайн")
                        };
                        ui.label(
                            RichText::new(format!("{} {}", dot, label))
                                .font(FontId::proportional(11.0))
                                .color(color),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Кнопка синхронизации
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🔄").font(FontId::proportional(14.0)),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Синхронизировать")
                            .clicked()
                        {
                            state.force_sync = true;
                        }
                        
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("✏").font(FontId::proportional(16.0)),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Новый контакт")
                            .clicked()
                        {
                            state.screen = crate::state::Screen::Contacts;
                        }
                    });
                });
            });

        // ── Поиск ──────────────────────────────────────────────────────
        Frame::none()
            .inner_margin(Margin::symmetric(12.0, 8.0))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.sidebar_search)
                        .hint_text("🔍 Поиск")
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(13.5)),
                );
            });

        ui.add(egui::Separator::default().spacing(0.0));

        // ── Список бесед ───────────────────────────────────────────────
        let search = state.sidebar_search.to_lowercase();

        egui::ScrollArea::vertical()
            .id_source("sidebar_scroll")
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());

                let items: Vec<_> = state
                    .conversations
                    .iter()
                    .filter(|c| {
                        search.is_empty()
                            || c.display_name.to_lowercase().contains(&search)
                            || c.last_preview.to_lowercase().contains(&search)
                    })
                    .cloned()
                    .collect();

                if items.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Нет бесед").color(theme::TEXT_TIMESTAMP));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Добавьте контакт чтобы начать")
                                .font(FontId::proportional(12.0))
                                .color(theme::TEXT_TIMESTAMP),
                        );
                    });
                }

                for item in &items {
                    let is_selected = state.selected_conv_id == Some(item.id);
                    if conversation_row(ui, item, is_selected).clicked() {
                        selected = Some(item.id);
                    }
                }
            });
    });

    selected
}

/// Рисует одну строку в списке бесед.
fn conversation_row(ui: &mut Ui, item: &ConversationItem, is_selected: bool) -> Response {
    let row_height = 64.0;
    let available_width = ui.available_width();

    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(available_width, row_height), Sense::click());

    if ui.is_rect_visible(rect) {
        let bg = if is_selected {
            theme::BG_SELECTED
        } else if response.hovered() {
            theme::BG_HOVER
        } else {
            Color32::TRANSPARENT
        };

        ui.painter().rect_filled(rect, Rounding::same(0.0), bg);

        // Левая полоска выделения
        if is_selected {
            let stripe = egui::Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height()));
            ui.painter()
                .rect_filled(stripe, Rounding::same(0.0), theme::ACCENT);
        }

        let inner = rect.shrink2(Vec2::new(12.0, 10.0));
        let painter = ui.painter();

        // Аватар
        let avatar_center = egui::pos2(inner.left() + 22.0, inner.center().y);
        avatar_circle_at(
            painter,
            avatar_center,
            22.0,
            item.avatar_letter,
            theme::BG_HOVER,
        );

        // Имя и превью
        let text_left = inner.left() + 52.0;
        let text_rect = egui::Rect::from_min_max(
            egui::pos2(text_left, inner.top()),
            egui::pos2(inner.right() - 48.0, inner.bottom()),
        );

        painter.text(
            egui::pos2(text_rect.left(), text_rect.top() + 4.0),
            Align2::LEFT_TOP,
            &item.display_name,
            FontId::proportional(14.0),
            theme::TEXT_PRIMARY,
        );

        let preview = if item.last_preview.len() > 42 {
            format!("{}…", &item.last_preview[..42])
        } else {
            item.last_preview.clone()
        };

        painter.text(
            egui::pos2(text_rect.left(), text_rect.top() + 24.0),
            Align2::LEFT_TOP,
            &preview,
            FontId::proportional(12.5),
            theme::TEXT_SECONDARY,
        );

        // Время и счётчик непрочитанных (правый край)
        if let Some(t) = item.last_msg_at {
            let local = Local.from_utc_datetime(&t.naive_utc());
            let time_str = if (chrono::Local::now() - local).num_hours() < 20 {
                local.format("%H:%M").to_string()
            } else {
                local.format("%d.%m").to_string()
            };

            painter.text(
                egui::pos2(inner.right() - 2.0, inner.top() + 4.0),
                Align2::RIGHT_TOP,
                &time_str,
                FontId::proportional(11.5),
                theme::TEXT_TIMESTAMP,
            );
        }

        // Бейдж непрочитанных
        if item.unread > 0 {
            let badge_center = egui::pos2(inner.right() - 10.0, inner.bottom() - 10.0);
            let badge_r = 9.0;
            painter.circle_filled(badge_center, badge_r, theme::ACCENT);
            painter.text(
                badge_center,
                Align2::CENTER_CENTER,
                &item.unread.to_string(),
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }

        // Разделитель
        let sep_y = rect.bottom() - 0.5;
        painter.line_segment(
            [
                egui::pos2(rect.left() + 60.0, sep_y),
                egui::pos2(rect.right(), sep_y),
            ],
            Stroke::new(0.5, theme::SEPARATOR),
        );
    }

    response
}

// ── Утилиты ───────────────────────────────────────────────────────────────────

/// Рисует круглый аватар-заглушку с буквой внутри egui Ui.
pub fn avatar_circle(ui: &mut Ui, letter: char, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    painter.circle_filled(center, size / 2.0, color.linear_multiply(0.3));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        &letter.to_string(),
        FontId::proportional(size * 0.45),
        color,
    );
}

fn avatar_circle_at(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    letter: char,
    bg: Color32,
) {
    painter.circle_filled(center, radius, bg);
    painter.text(
        center,
        Align2::CENTER_CENTER,
        &letter.to_string(),
        FontId::proportional(radius * 0.9),
        theme::ACCENT,
    );
}
