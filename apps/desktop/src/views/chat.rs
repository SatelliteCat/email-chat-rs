//! Область сообщений — история чата со скроллом.

use chrono::{Local, TimeZone};
use egui::{
    Align, Align2, Color32, FontId, Frame, Layout, Margin, Pos2,
    RichText, Rounding, ScrollArea, Stroke, Ui,
};
use uuid::Uuid;

use core::models::message::{Message, MessageStatus};

use crate::{
    runtime::{AppEvent, EventSender},
    state::{ChatUiState, ConversationItem},
    views::theme,
};

/// Рисует заголовок беседы + область сообщений.
pub fn show(
    ui: &mut Ui,
    chat: &mut ChatUiState,
    conv: &ConversationItem,
    my_email: &str,
    sender: &EventSender,
) {
    chat_header(ui, conv, sender);
    ui.add(egui::Separator::default().spacing(0.0));
    messages_area(ui, chat, my_email);
}

fn chat_header(ui: &mut Ui, conv: &ConversationItem, sender: &EventSender) {
    Frame::none()
        .fill(theme::BG_PANEL)
        .inner_margin(Margin::symmetric(16.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                super::sidebar::avatar_circle(ui, conv.avatar_letter, 36.0, theme::ACCENT);
                ui.add_space(10.0);

                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&conv.display_name)
                            .font(FontId::proportional(15.0))
                            .color(theme::TEXT_PRIMARY)
                            .strong(),
                    );
                    ui.label(
                        RichText::new("🔒 E2E шифрование")
                            .font(FontId::proportional(11.0))
                            .color(theme::SUCCESS),
                    );
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🗑").font(FontId::proportional(16.0)),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Удалить беседу (с сервера)")
                        .clicked()
                    {
                        sender.send(AppEvent::DeleteConversation { conv_id: conv.id });
                    }
                });
            });
        });
}

fn messages_area(ui: &mut Ui, chat: &mut ChatUiState, my_email: &str) {
    ScrollArea::vertical()
        .id_source("chat_scroll")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            if chat.is_loading_history {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Загрузка…").color(theme::TEXT_TIMESTAMP));
                });
                return;
            }

            if chat.messages.is_empty() {
                ui.add_space(80.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🔒").font(FontId::proportional(36.0)));
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Переписка зашифрована end-to-end")
                            .color(theme::TEXT_SECONDARY),
                    );
                    ui.label(
                        RichText::new("Сообщения видны только вам и собеседнику")
                            .font(FontId::proportional(12.0))
                            .color(theme::TEXT_TIMESTAMP),
                    );
                });
                return;
            }

            let available_w = ui.available_width();
            let mut last_date: Option<chrono::NaiveDate> = None;

            for msg in &chat.messages {
                let local = Local.from_utc_datetime(&msg.sent_at.naive_utc());
                let date = local.date_naive();

                if last_date != Some(date) {
                    last_date = Some(date);
                    day_separator(ui, &local.format("%d %B %Y").to_string(), available_w);
                }

                let is_mine = msg.from_email.eq_ignore_ascii_case(my_email);
                message_bubble(ui, msg, is_mine);
            }

            ui.add_space(8.0);
        });
}

fn day_separator(ui: &mut Ui, label: &str, width: f32) {
    ui.add_space(10.0);
    let font = FontId::proportional(11.5);
    let color = theme::TEXT_TIMESTAMP;

    ui.vertical_centered(|ui| {
        ui.label(RichText::new(label).font(font).color(color));
    });
    ui.add_space(4.0);
}

fn message_bubble(ui: &mut Ui, msg: &Message, is_mine: bool) {
    let max_w = (ui.available_width() * 0.72).max(200.0);
    let text = msg.body.as_deref().unwrap_or("[пустое]");
    let local = Local.from_utc_datetime(&msg.sent_at.naive_utc());
    let time_str = local.format("%H:%M").to_string();

    let status_icon = if is_mine {
        match msg.status {
            MessageStatus::Queued    => " ⏳",
            MessageStatus::Sending   => " ⌛",
            MessageStatus::Sent      => " ✓",
            MessageStatus::Delivered => " ✓✓",
            MessageStatus::Read      => " ✓✓",
        }
    } else {
        ""
    };

    let layout = if is_mine {
        Layout::right_to_left(Align::TOP)
    } else {
        Layout::left_to_right(Align::TOP)
    };

    ui.with_layout(layout, |ui| {
        ui.add_space(12.0);

        Frame::none()
            .fill(if is_mine { theme::BG_MSG_OUT } else { theme::BG_MSG_IN })
            .rounding(Rounding {
                nw: if is_mine { 12.0 } else { 3.0 },
                ne: if is_mine { 3.0 } else { 12.0 },
                sw: 12.0,
                se: 12.0,
            })
            .inner_margin(Margin::symmetric(12.0, 8.0))
            .show(ui, |ui| {
                ui.set_max_width(max_w);

                if !is_mine {
                    ui.label(
                        RichText::new(&msg.from_email)
                            .font(FontId::proportional(11.5))
                            .color(theme::ACCENT),
                    );
                }

                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .font(FontId::proportional(14.5))
                            .color(theme::TEXT_PRIMARY),
                    )
                    .wrap(true),
                );

                ui.with_layout(Layout::right_to_left(Align::BOTTOM), |ui| {
                    ui.label(
                        RichText::new(format!("{}{}", time_str, status_icon))
                            .font(FontId::proportional(10.5))
                            .color(theme::TEXT_TIMESTAMP),
                    );
                });
            });

        ui.add_space(12.0);
    });

    ui.add_space(3.0);
}
