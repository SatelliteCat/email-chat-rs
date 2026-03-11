//! Поле ввода сообщения + кнопка отправить.

use egui::{Color32, FontId, Frame, Key, Margin, Modifiers, RichText, Ui, Vec2};

use crate::views::theme;

/// Результат взаимодействия с полем ввода.
pub enum ComposeAction {
    Send(String),
    Nothing,
}

/// Рисует поле ввода. Возвращает `Send(text)` если пользователь нажал отправить.
pub fn show(ui: &mut Ui, draft: &mut String, is_sending: bool) -> ComposeAction {
    let mut action = ComposeAction::Nothing;

    Frame::none()
        .fill(theme::BG_DARK)
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Поле ввода
                let text_edit = egui::TextEdit::multiline(draft)
                    .hint_text("Введите сообщение… (Enter — отправить, Shift+Enter — перенос)")
                    .desired_width(ui.available_width() - 56.0)
                    .desired_rows(1)
                    .font(FontId::proportional(14.5))
                    .lock_focus(true);

                let response = ui.add(text_edit);

                // Enter без Shift → отправить
                let send_by_key = response.has_focus()
                    && ui.input(|i| {
                        i.key_pressed(Key::Enter) && !i.modifiers.shift
                    });

                // Удаляем одиночный перенос который Enter добавил
                if send_by_key && draft.ends_with('\n') {
                    draft.pop();
                }

                let can_send = !draft.trim().is_empty() && !is_sending;

                if send_by_key && can_send {
                    let text = draft.trim().to_string();
                    draft.clear();
                    action = ComposeAction::Send(text);
                }

                // Кнопка отправить
                let btn_label = if is_sending { "⌛" } else { "➤" };
                let send_btn = egui::Button::new(
                    RichText::new(btn_label)
                        .font(FontId::proportional(18.0))
                        .color(Color32::WHITE),
                )
                .fill(if can_send { theme::ACCENT } else { theme::BG_HOVER })
                .min_size(Vec2::splat(40.0));

                if ui.add_enabled(can_send, send_btn).clicked() {
                    let text = draft.trim().to_string();
                    draft.clear();
                    action = ComposeAction::Send(text);
                }
            });
        });

    action
}
