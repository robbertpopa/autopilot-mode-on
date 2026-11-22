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

// The tokens of the "Autopilot Redesign" canvas. `AMBER`, `GREEN` and `RED`
// are text shades: in the light palette they are darker than the canvas fills,
// because a bright fill colour on white fails 4.5:1 as text.
colors! {
    BACKGROUND: Color32::from_rgb(0x0F, 0x10, 0x12), Color32::from_rgb(0xF4, 0xF5, 0xF7);
    SIDEBAR: Color32::from_rgb(0x13, 0x14, 0x17), Color32::from_rgb(0xEC, 0xEE, 0xF2);
    CARD: Color32::from_rgb(0x17, 0x18, 0x1B), Color32::from_rgb(0xFF, 0xFF, 0xFF);
    NODE: Color32::from_rgb(0x1E, 0x20, 0x24), Color32::from_rgb(0xEE, 0xF0, 0xF3);
    FIELD: Color32::from_rgb(0x0F, 0x10, 0x12), Color32::from_rgb(0xFF, 0xFF, 0xFF);
    SELECTED: Color32::from_rgb(0x26, 0x28, 0x2D), Color32::from_rgb(0xE3, 0xE6, 0xEB);
    BORDER: Color32::from_rgb(0x2A, 0x2C, 0x31), Color32::from_rgb(0xDC, 0xDF, 0xE5);
    BORDER_STRONG: Color32::from_rgb(0x33, 0x36, 0x3C), Color32::from_rgb(0xC9, 0xCE, 0xD6);
    TEXT: Color32::from_rgb(0xEC, 0xED, 0xEF), Color32::from_rgb(0x15, 0x18, 0x1D);
    TEXT_SOFT: Color32::from_rgb(0xC4, 0xC7, 0xCD), Color32::from_rgb(0x3A, 0x41, 0x4B);
    TEXT_MUTED: Color32::from_rgb(0x93, 0x99, 0xA3), Color32::from_rgb(0x5F, 0x67, 0x73);
    ARROW: Color32::from_rgb(0x6B, 0x70, 0x79), Color32::from_rgb(0x8A, 0x92, 0x9D);
    BLUE: Color32::from_rgb(0x6E, 0xA2, 0xFF), Color32::from_rgb(0x2F, 0x6F, 0xE4);
    BLUE_TEXT: Color32::from_rgb(0xA9, 0xC9, 0xFF), Color32::from_rgb(0x1D, 0x4F, 0xA8);
    BLUE_FILL: Color32::from_rgb(0x13, 0x23, 0x3D), Color32::from_rgb(0xE2, 0xEC, 0xFF);
    ON_ACCENT: Color32::from_rgb(0x0B, 0x12, 0x20), Color32::from_rgb(0xFF, 0xFF, 0xFF);
    GREEN: Color32::from_rgb(0x4C, 0xC3, 0x8A), Color32::from_rgb(0x1E, 0x7A, 0x4E);
    GREEN_FILL: Color32::from_rgb(0x14, 0x2A, 0x1F), Color32::from_rgb(0xDD, 0xF3, 0xE8);
    AMBER: Color32::from_rgb(0xF0, 0xB9, 0x4D), Color32::from_rgb(0x8A, 0x56, 0x00);
    AMBER_SOLID: Color32::from_rgb(0xF0, 0xB9, 0x4D), Color32::from_rgb(0xE9, 0xA9, 0x3A);
    RED_SOLID: Color32::from_rgb(0xF0, 0x77, 0x7A), Color32::from_rgb(0xD9, 0x47, 0x4B);
    GREEN_SOLID: Color32::from_rgb(0x4C, 0xC3, 0x8A), Color32::from_rgb(0x2E, 0x9E, 0x66);
    AMBER_FILL: Color32::from_rgb(0x2B, 0x21, 0x10), Color32::from_rgb(0xFF, 0xF1, 0xD6);
    RED: Color32::from_rgb(0xF5, 0x9A, 0x9C), Color32::from_rgb(0xB4, 0x2F, 0x33);
    RED_FILL: Color32::from_rgb(0x2E, 0x16, 0x18), Color32::from_rgb(0xFD, 0xE3, 0xE3);
    GREY_FILL: Color32::from_rgb(0x26, 0x28, 0x2D), Color32::from_rgb(0xE3, 0xE6, 0xEB);
    GREY_TEXT: Color32::from_rgb(0xB5, 0xBA, 0xC2), Color32::from_rgb(0x4A, 0x52, 0x5D);
}

/// The owner's choice. `System` follows the operating system and is checked
/// again every frame, because the colour functions read one stored flag.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    /// Read the saved word. Unknown text falls back to `System` so a damaged
    /// file never stops the window from opening.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// True for the light palette. With no answer from the system the window
    /// stays dark, the palette it had before there was a choice.
    #[must_use]
    pub fn is_light(self, system: Option<egui::Theme>) -> bool {
        match self {
            Self::Light => true,
            Self::Dark => false,
            Self::System => system == Some(egui::Theme::Light),
        }
    }
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
    let radius = CornerRadius::same(7);
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
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.text_styles = [
            (
                egui::TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Body,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Button,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Heading,
                FontId::new(18.0, FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Monospace,
                FontId::new(12.0, FontFamily::Monospace),
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
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
}

/// A small coloured label such as "Needs you" or "always runs".
pub fn pill(ui: &mut Ui, text: &str, fill: Color32, color: Color32) {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).monospace().size(11.0).color(color));
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
                        .color(ON_ACCENT()),
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
    egui::Button::new(RichText::new(text).strong().size(13.0).color(ON_ACCENT()))
        .fill(BLUE())
        .min_size(egui::vec2(0.0, 34.0))
}

/// A segmented switch. Returns true if the choice changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, current: &mut T, options: &[(T, &str)]) -> bool {
    let before = *current;
    Frame::new()
        .stroke(Stroke::new(1.0_f32, BORDER()))
        .corner_radius(CornerRadius::same(7))
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
