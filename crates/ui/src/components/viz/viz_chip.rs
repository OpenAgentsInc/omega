use documented::Documented;
use gpui::px;

use crate::components::viz::VizPalette;
use crate::prelude::*;

/// The emphasis tone of a record chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizChipTone {
    Neutral,
    Active,
    Ok,
    Warn,
}

impl VizChipTone {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Active => "active",
            Self::Ok => "ok",
            Self::Warn => "warn",
        }
    }

    pub fn color(&self, palette: &VizPalette) -> gpui::Hsla {
        match self {
            Self::Neutral => palette.muted,
            Self::Active => palette.giftwrap,
            Self::Ok => palette.ok,
            Self::Warn => palette.warn,
        }
    }
}

/// A protocol record rendered as a monospace pill — data travels the scene as
/// a visible satellite chip, not a tooltip. The Nostr kind number renders as a
/// muted prefix (`39605 Quote`).
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct VizChip {
    kind: Option<u32>,
    label: SharedString,
    tone: VizChipTone,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizChip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            kind: None,
            label: label.into(),
            tone: VizChipTone::Neutral,
            scale: 1.5,
            palette: None,
        }
    }

    /// The Nostr kind number rendered as a muted prefix.
    pub fn kind(mut self, kind: u32) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn tone(mut self, tone: VizChipTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl RenderOnce for VizChip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let tone = self.tone.color(&palette);

        h_flex()
            .gap(px(3.0 * scale))
            .px(px(6.0 * scale))
            .h(px(15.0 * scale))
            .rounded_full()
            .border_1()
            .border_color(tone)
            .bg(palette.node_fill)
            .font_buffer(cx)
            .text_size(px(8.5 * scale))
            .when_some(self.kind, |this, kind| {
                this.child(div().text_color(palette.muted).child(kind.to_string()))
            })
            .child(div().text_color(palette.node_text).child(self.label))
    }
}

impl Component for VizChip {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let records: [(u32, &str, VizChipTone); 4] = [
            (39604, "RFQ", VizChipTone::Neutral),
            (39605, "Quote", VizChipTone::Active),
            (39610, "Contract", VizChipTone::Ok),
            (39607, "Status", VizChipTone::Warn),
        ];
        let row = |palette: Option<VizPalette>| {
            h_flex()
                .gap_2()
                .children(records.iter().map(|(kind, label, tone)| {
                    let mut chip = VizChip::new(*label).kind(*kind).tone(*tone);
                    if let Some(palette) = palette {
                        chip = chip.palette(palette);
                    }
                    chip
                }))
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Record chips",
                vec![single_example("Kinds and tones", row(None))],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Tone rides the border, meaning stays in the text",
                    row(Some(VizPalette::from_theme(cx).grayscale())),
                )],
            ))
            .into_any_element()
    }
}
