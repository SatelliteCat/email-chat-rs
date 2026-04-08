pub mod chat;
pub mod compose;
pub mod contacts;
pub mod conversation_settings;
pub mod login;
pub mod sidebar;

/// Цветовая палитра приложения.
pub mod theme {
    use egui::Color32;

    pub const BG_DARK: Color32        = Color32::from_rgb(18,  18,  24);
    pub const BG_PANEL: Color32       = Color32::from_rgb(26,  26,  34);
    pub const BG_HOVER: Color32       = Color32::from_rgb(38,  38,  50);
    pub const BG_SELECTED: Color32    = Color32::from_rgb(48,  90, 160);
    pub const BG_MSG_OUT: Color32     = Color32::from_rgb(37,  80, 145);
    pub const BG_MSG_IN: Color32      = Color32::from_rgb(40,  40,  52);

    pub const TEXT_PRIMARY: Color32   = Color32::from_rgb(220, 220, 230);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(140, 140, 160);
    pub const TEXT_TIMESTAMP: Color32 = Color32::from_rgb(100, 100, 120);

    pub const ACCENT: Color32         = Color32::from_rgb(80, 140, 255);
    pub const ACCENT_HOVER: Color32   = Color32::from_rgb(100, 160, 255);
    pub const SUCCESS: Color32        = Color32::from_rgb(80, 200, 120);
    pub const ERROR: Color32          = Color32::from_rgb(220,  80,  80);

    pub const SEPARATOR: Color32      = Color32::from_rgb(45,  45,  58);

    /// Применяет тёмную тему к egui context.
    pub fn apply(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();

        visuals.window_fill         = BG_DARK;
        visuals.panel_fill          = BG_PANEL;
        visuals.faint_bg_color      = BG_HOVER;
        visuals.extreme_bg_color    = BG_DARK;

        visuals.widgets.noninteractive.bg_fill  = BG_PANEL;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(0.0, TEXT_PRIMARY);
        visuals.widgets.inactive.bg_fill        = BG_PANEL;
        visuals.widgets.hovered.bg_fill         = BG_HOVER;
        visuals.widgets.active.bg_fill          = BG_SELECTED;

        visuals.selection.bg_fill   = BG_SELECTED;
        visuals.selection.stroke    = egui::Stroke::new(0.0, TEXT_PRIMARY);

        visuals.window_rounding     = egui::Rounding::same(8.0);
        visuals.menu_rounding       = egui::Rounding::same(6.0);

        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing   = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.window_margin  = egui::Margin::same(0.0);
        ctx.set_style(style);
    }
}
