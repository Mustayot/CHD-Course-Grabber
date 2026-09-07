//! 主题与通用 UI 部件:中文字体注入 + 浅/深两套配色 + 常用卡片/chip 控件。
use crate::model::Lvl;
use eframe::egui;

pub const ACCENT: (u8, u8, u8) = (0x1B, 0x6F, 0xE0); // 主题蓝
pub const ACCENT_LIGHT: (u8, u8, u8) = (0x5B, 0x9E, 0xF5);

pub struct Palette {
    pub bg: egui::Color32,
    pub panel: egui::Color32,
    pub card: egui::Color32,
    pub card_alt: egui::Color32,
    pub text: egui::Color32,
    pub dim: egui::Color32,
    pub accent: egui::Color32,
    pub accent_text: egui::Color32,
    pub border: egui::Color32,
    pub ok: egui::Color32,
    pub warn: egui::Color32,
    pub err: egui::Color32,
    pub succ: egui::Color32,
    pub info: egui::Color32,
    pub head_bg: egui::Color32,
    pub head_fg: egui::Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: egui::Color32::from_rgb(18, 20, 26),
            panel: egui::Color32::from_rgb(22, 25, 33),
            card: egui::Color32::from_rgb(28, 31, 41),
            card_alt: egui::Color32::from_rgb(34, 38, 50),
            text: egui::Color32::from_rgb(232, 234, 240),
            dim: egui::Color32::from_rgb(148, 154, 170),
            accent: egui::Color32::from_rgb(ACCENT_LIGHT.0, ACCENT_LIGHT.1, ACCENT_LIGHT.2),
            accent_text: egui::Color32::WHITE,
            border: egui::Color32::from_rgb(52, 57, 72),
            ok: egui::Color32::from_rgb(110, 210, 130),
            warn: egui::Color32::from_rgb(245, 190, 90),
            err: egui::Color32::from_rgb(240, 110, 110),
            succ: egui::Color32::from_rgb(120, 225, 160),
            info: egui::Color32::from_rgb(140, 175, 235),
            head_bg: egui::Color32::from_rgb(24, 27, 36),
            head_fg: egui::Color32::from_rgb(232, 234, 240),
        }
    } else {
        Palette {
            bg: egui::Color32::from_rgb(244, 246, 250),
            panel: egui::Color32::from_rgb(236, 239, 245),
            card: egui::Color32::from_rgb(255, 255, 255),
            card_alt: egui::Color32::from_rgb(249, 250, 252),
            text: egui::Color32::from_rgb(30, 34, 43),
            dim: egui::Color32::from_rgb(120, 127, 142),
            accent: egui::Color32::from_rgb(ACCENT.0, ACCENT.1, ACCENT.2),
            accent_text: egui::Color32::WHITE,
            border: egui::Color32::from_rgb(224, 228, 236),
            ok: egui::Color32::from_rgb(34, 158, 72),
            warn: egui::Color32::from_rgb(204, 138, 0),
            err: egui::Color32::from_rgb(213, 53, 65),
            succ: egui::Color32::from_rgb(23, 150, 60),
            info: egui::Color32::from_rgb(37, 100, 200),
            head_bg: egui::Color32::from_rgb(0x14, 0x55, 0xA8),
            head_fg: egui::Color32::WHITE,
        }
    }
}

pub fn setup(ctx: &egui::Context) {
    install_cjk_fonts(ctx);
    apply_visuals(ctx, false);
}

fn install_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/msyhbd.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/msyhl.ttc",
        "C:/Windows/Fonts/simsun.ttc",
    ];
    let mut inserted = false;
    for p in candidates {
        if let Ok(bytes) = std::fs::read(p) {
            fonts.font_data.insert("cjk".into(), egui::FontData::from_owned(bytes));
            inserted = true;
            break;
        }
    }
    if inserted {
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&fam) {
                list.push("cjk".into());
            }
        }
        ctx.set_fonts(fonts);
    }
}

fn apply_visuals(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.dark_mode = dark;
    v.panel_fill = p.panel;
    v.window_fill = p.card;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.card_alt;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.linear_multiply(0.6);
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.rounding = egui::Rounding::same(8.0);
        w.bg_stroke = egui::Stroke::new(1.0, p.border);
    }
    v.widgets.inactive.bg_fill = p.card;
    v.widgets.hovered.bg_fill = p.card_alt;
    v.widgets.active.bg_fill = p.accent.linear_multiply(0.25);
    v.widgets.hovered.weak_bg_fill = p.card_alt;
    v.window_stroke = egui::Stroke::new(1.0, p.border);
    ctx.set_visuals(v);
    // 圆角更明显的按钮/输入框
    let mut style = (*ctx.style()).clone();
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(8.0);
    ctx.set_style(style);
}

pub fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let p = palette_cur(ui);
    egui::Frame::none()
        .fill(p.card)
        .stroke(egui::Stroke::new(1.0, p.border))
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(title).size(15.0).strong().color(p.accent));
                });
                ui.add_space(6.0);
            }
            add(ui);
        });
}

pub fn palette_cur(ui: &egui::Ui) -> Palette {
    palette(ui.ctx().style().visuals.dark_mode)
}

pub fn chip(ui: &mut egui::Ui, text: &str, fg: egui::Color32, bg: egui::Color32) {
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.0).strong().color(fg));
        });
}

pub fn lvl_color(lvl: Lvl, dark: bool) -> egui::Color32 {
    let p = palette(dark);
    match lvl {
        Lvl::Info => p.info,
        Lvl::Ok => p.ok,
        Lvl::Warn => p.warn,
        Lvl::Err => p.err,
        Lvl::Succ => p.succ,
    }
}

pub fn tint(c: egui::Color32, alpha: u8) -> egui::Color32 {
    let (r, g, b, _) = (c.r(), c.g(), c.b(), c.a());
    egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// 一个带强调色的主按钮
pub fn accent_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = palette_cur(ui);
    let btn = egui::Button::new(egui::RichText::new(text).size(14.0).strong().color(p.accent_text))
        .fill(p.accent)
        .stroke(egui::Stroke::new(1.0, p.accent))
        .min_size(egui::vec2(0.0, 34.0));
    ui.add(btn)
}

pub fn soft_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = palette_cur(ui);
    let btn = egui::Button::new(egui::RichText::new(text).size(13.0).color(p.text))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, p.border))
        .min_size(egui::vec2(0.0, 30.0));
    ui.add(btn)
}

/// 状态文字颜色映射(抢课目标状态)
pub fn state_color(s: &crate::model::TState, dark: bool) -> egui::Color32 {
    use crate::model::TState::*;
    let p = palette(dark);
    match s {
        Idle => p.dim,
        Grabbing => p.info,
        Won => p.succ,
        Full => p.warn,
        Conflict => p.warn,
        Blocked => p.err,
        SessionLost => p.err,
    }
}

pub fn state_label(s: &crate::model::TState) -> &'static str {
    use crate::model::TState::*;
    match s {
        Idle => "待命",
        Grabbing => "抢课中…",
        Won => "已抢中",
        Full => "暂时满员",
        Conflict => "冲突/停手",
        Blocked => "不可选",
        SessionLost => "会话失效",
    }
}
