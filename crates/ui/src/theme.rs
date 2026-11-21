//! Colours and small widgets shared by every view. The palette is the one in
//! the design boards.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, RichText, Stroke, Ui,
};

static LIGHT: AtomicBool = AtomicBool::new(false);

/// Choose the palette every colour function below answers with.
pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

#[must_use]
pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

macro_rules! colors {
    ($($name:ident: $dark:expr, $light:expr;)*) => {
        $(
            #[allow(non_snake_case, reason = "reads like the constant it replaces")]
            #[must_use]
            pub fn $name() -> Color32 {
                if is_light() { $light } else { $dark }
            }
        )*
    };
}

colors! {
    BACKGROUND: Color32::from_rgb(0x0E, 0x0F, 0x11), Color32::from_rgb(0xF6, 0xF7, 0xF9);
    TITLE_BAR: Color32::from_rgb(0x0A, 0x0B, 0x0D), Color32::from_rgb(0xFF, 0xFF, 0xFF);
    SIDEBAR: Color32::from_rgb(0x12, 0x13, 0x16), Color32::from_rgb(0xEE, 0xF0, 0xF3);
    CARD: Color32::from_rgb(0x17, 0x19, 0x1C), Color32::from_rgb(0xFF, 0xFF, 0xFF);
    NODE: Color32::from_rgb(0x1D, 0x20, 0x24), Color32::from_rgb(0xF2, 0xF4, 0xF7);
    FIELD: Color32::from_rgb(0x14, 0x16, 0x1A), Color32::from_rgb(0xFF, 0xFF, 0xFF);
    SELECTED: Color32::from_rgb(0x22, 0x25, 0x2A), Color32::from_rgb(0xE3, 0xE7, 0xED);
    BORDER: Color32::from_rgb(0x2A, 0x2D, 0x33), Color32::from_rgb(0xD9, 0xDD, 0xE3);
    BORDER_STRONG: Color32::from_rgb(0x3A, 0x3F, 0x47), Color32::from_rgb(0xC3, 0xC9, 0xD2);
    TEXT: Color32::from_rgb(0xE8, 0xEA, 0xED), Color32::from_rgb(0x15, 0x18, 0x1D);
    TEXT_SOFT: Color32::from_rgb(0xC9, 0xCE, 0xD6), Color32::from_rgb(0x3A, 0x41, 0x4B);
    TEXT_MUTED: Color32::from_rgb(0x9A, 0xA1, 0xAB), Color32::from_rgb(0x6A, 0x72, 0x7D);
    ARROW: Color32::from_rgb(0x6C, 0x74, 0x80), Color32::from_rgb(0x8A, 0x92, 0x9D);
    BLUE: Color32::from_rgb(0x5B, 0x9B, 0xFF), Color32::from_rgb(0x2F, 0x6F, 0xE4);
    BLUE_TEXT: Color32::from_rgb(0xA9, 0xC9, 0xFF), Color32::from_rgb(0x1D, 0x4F, 0xA8);
    BLUE_FILL: Color32::from_rgb(0x13, 0x23, 0x3D), Color32::from_rgb(0xE2, 0xEC, 0xFF);
    GREEN: Color32::from_rgb(0x4C, 0xC3, 0x8A), Color32::from_rgb(0x1E, 0x8F, 0x5C);
    GREEN_FILL: Color32::from_rgb(0x14, 0x29, 0x1F), Color32::from_rgb(0xDD, 0xF3, 0xE8);
    AMBER: Color32::from_rgb(0xF0, 0xB9, 0x4D), Color32::from_rgb(0xA8, 0x6A, 0x00);
    AMBER_FILL: Color32::from_rgb(0x2A, 0x21, 0x10), Color32::from_rgb(0xFF, 0xF1, 0xD6);
    RED: Color32::from_rgb(0xF0, 0x77, 0x7A), Color32::from_rgb(0xC9, 0x3A, 0x3E);
    RED_FILL: Color32::from_rgb(0x2E, 0x16, 0x18), Color32::from_rgb(0xFD, 0xE3, 0xE3);
    GREY_FILL: Color32::from_rgb(0x26, 0x2A, 0x30), Color32::from_rgb(0xE6, 0xE9, 0xEE);
    GREY_TEXT: Color32::from_rgb(0xAE, 0xB4, 0xBE), Color32::from_rgb(0x52, 0x5A, 0x66);
    LIGHT_BUTTON: Color32::from_rgb(0xF3, 0xF4, 0xF6), Color32::from_rgb(0x15, 0x18, 0x1D);
    DARK_ON_LIGHT: Color32::from_rgb(0x11, 0x13, 0x18), Color32::from_rgb(0xFF, 0xFF, 0xFF);
}

/// The product icon, shared by the window and the title bar.
pub const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");

/// The icon as a texture for the title bar.
#[must_use]
pub fn icon_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG).ok()?;
    let size = [
        usize::try_from(icon.width).ok()?,
        usize::try_from(icon.height).ok()?,
    ];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
    Some(ctx.load_texture("icon", image, egui::TextureOptions::LINEAR))
}

/// Inter (SIL Open Font License, text in `assets/fonts`) in front of egui's
/// own fonts, which stay behind it for the symbols Inter lacks.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/Inter.ttf"
        ))),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, "Inter".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// The chosen palette, rounded corners and the design's text sizes.
pub fn apply(ctx: &egui::Context, light: bool) {
    set_light(light);
    install_fonts(ctx);
    let mut visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.panel_fill = BACKGROUND();
    visuals.window_fill = CARD();
    visuals.extreme_bg_color = FIELD();
    visuals.override_text_color = None;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT());
    visuals.widgets.hovered.fg_stroke =
        Stroke::new(1.5_f32, if light { TEXT() } else { Color32::WHITE });
    visuals.widgets.active.fg_stroke =
        Stroke::new(1.5_f32, if light { TEXT() } else { Color32::WHITE });
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT());
    visuals.selection.bg_fill = BLUE();
    visuals.hyperlink_color = BLUE();
    let radius = CornerRadius::same(9);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }
    visuals.widgets.inactive.bg_fill = NODE();
    visuals.widgets.inactive.weak_bg_fill = NODE();
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG());
    visuals.widgets.hovered.weak_bg_fill = SELECTED();
    visuals.widgets.hovered.bg_fill = SELECTED();
    ctx.set_theme(if light {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.text_styles = [
            (
                egui::TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Body,
                FontId::new(15.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Button,
                FontId::new(15.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Heading,
                FontId::new(26.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// A rounded panel with a border, like the cards in the boards.
pub fn card(fill: Color32, border: Color32) -> Frame {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
}

/// A small coloured label such as "Needs you" or "always runs".
pub fn pill(ui: &mut Ui, text: &str, fill: Color32, color: Color32) {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).strong().color(color));
        });
}

/// The `API $` or `SUB` tag that tells exact spend from an estimate.
pub fn cost_tag(ui: &mut Ui, estimated: bool) {
    if estimated {
        Frame::new()
            .stroke(Stroke::new(1.0_f32, TEXT_MUTED()))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(5, 0))
            .show(ui, |ui| {
                ui.label(RichText::new("SUB").size(10.5).strong().color(TEXT_SOFT()));
            });
    } else {
        Frame::new()
            .fill(BLUE())
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(5, 0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("API $")
                        .size(10.5)
                        .strong()
                        .color(Color32::from_rgb(0x0B, 0x12, 0x20)),
                );
            });
    }
}

#[must_use]
pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(TEXT_MUTED())
}

#[must_use]
pub fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().color(TEXT_SOFT())
}

/// Give a button the name a screen reader says, for controls whose visible
/// text is only a symbol.
#[must_use]
pub fn named(response: egui::Response, name: impl Into<String>) -> egui::Response {
    let name = name.into();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    response
}

/// A button for the one main action of a screen.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text)
            .strong()
            .size(16.0)
            .color(DARK_ON_LIGHT()),
    )
    .fill(LIGHT_BUTTON())
    .min_size(egui::vec2(0.0, 38.0))
}

/// A segmented switch. Returns true if the choice changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, current: &mut T, options: &[(T, &str)]) -> bool {
    let before = *current;
    Frame::new()
        .stroke(Stroke::new(1.0_f32, BORDER()))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.spacing_mut().button_padding.x = 10.0;
                for (value, label) in options {
                    let chosen = *current == *value;
                    let button = egui::Button::new(RichText::new(*label).color(if chosen {
                        TEXT()
                    } else {
                        TEXT_SOFT()
                    }))
                    .fill(if chosen {
                        SELECTED()
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE);
                    if ui.add(button).clicked() {
                        *current = *value;
                    }
                }
            });
        });
    before != *current
}
