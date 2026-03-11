//! Экран логина — добавление почтового аккаунта.

use egui::{Align, Color32, FontId, Frame, Layout, Margin, RichText, Rounding, Ui, Vec2};

use crate::{
    runtime::{AppEvent, EventSender},
    state::LoginState,
    views::theme,
};

/// Рисует экран добавления аккаунта.
pub fn show(ui: &mut Ui, state: &mut LoginState, sender: &EventSender) {
    let available = ui.available_size();

    ui.allocate_ui_with_layout(available, Layout::top_down(Align::Center), |ui| {
        ui.add_space(available.y * 0.22);

        Frame::none()
            .fill(theme::BG_PANEL)
            .rounding(Rounding::same(12.0))
            .inner_margin(Margin::same(32.0))
            .show(ui, |ui| {
                ui.set_max_width(380.0);

                // Заголовок
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("🔒 EChat")
                            .font(FontId::proportional(28.0))
                            .color(theme::ACCENT),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Зашифрованный чат через e-mail")
                            .color(theme::TEXT_SECONDARY),
                    );
                });

                ui.add_space(24.0);
                ui.separator();
                ui.add_space(16.0);

                // Email
                ui.label(RichText::new("Email").color(theme::TEXT_SECONDARY));
                ui.add_space(4.0);
                let email_resp = ui.add(
                    egui::TextEdit::singleline(&mut state.email)
                        .hint_text("user@mail.ru или user@yandex.ru")
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(15.0)),
                );

                ui.add_space(12.0);

                // Пароль приложения
                ui.label(RichText::new("Пароль приложения").color(theme::TEXT_SECONDARY));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.password)
                            .hint_text("Пароль для внешних приложений")
                            .password(!state.show_password)
                            .desired_width(ui.available_width() - 36.0)
                            .font(FontId::proportional(15.0)),
                    );
                    let eye = if state.show_password { "👁" } else { "🙈" };
                    if ui.small_button(eye).clicked() {
                        state.show_password = !state.show_password;
                    }
                });

                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Создайте пароль приложения в настройках Mail.ru или Яндекс.\n\
                         Основной пароль от почты не подойдёт.",
                    )
                    .font(FontId::proportional(11.5))
                    .color(theme::TEXT_TIMESTAMP),
                );

                ui.add_space(20.0);

                // Блок ошибки
                if let Some(err) = &state.error.clone() {
                    Frame::none()
                        .fill(Color32::from_rgb(60, 20, 20))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(format!("⚠ {}", err)).color(theme::ERROR));
                        });
                    ui.add_space(12.0);
                }

                // Кнопка
                ui.vertical_centered(|ui| {
                    let label = if state.is_loading { "Подключение…" } else { "Войти" };
                    let btn = egui::Button::new(
                        RichText::new(label)
                            .font(FontId::proportional(15.0))
                            .color(Color32::WHITE),
                    )
                    .fill(theme::ACCENT)
                    .min_size(Vec2::new(200.0, 40.0));

                    let can_submit =
                        !state.email.is_empty() && !state.password.is_empty() && !state.is_loading;

                    let enter_pressed = email_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    if ui.add_enabled(can_submit, btn).clicked()
                        || (enter_pressed && can_submit)
                    {
                        state.is_loading = true;
                        state.error = None;
                        let email = state.email.trim().to_lowercase();
                        let password = state.password.clone();
                        sender.send(AppEvent::LoginAttempt { email, password });
                    }
                });
            });
    });
}
