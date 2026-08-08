//! Market visualization primitives.
//!
//! A GPUI port of the Bazaar web client's viz language
//! (`bazaar/components/viz/core/` and `bazaar/docs/network-visualization-spec.md`).
//! The load-bearing rule, shared with `omega_status_cue`: color repeats a
//! meaning, it never carries it alone — every node state and edge class pairs
//! a color with a dash pattern, glyph, or stroke shape, so scenes survive
//! grayscale and color-vision deficiencies. See omega#247.

mod viz_chip;
mod viz_edge;
mod viz_geometry;
mod viz_node;
mod viz_port;
mod viz_progress_rail;
mod viz_zone;

pub use viz_chip::*;
pub use viz_edge::*;
pub use viz_geometry::*;
pub use viz_node::*;
pub use viz_port::*;
pub use viz_progress_rail::*;
pub use viz_zone::*;

use gpui::{App, Font, Hsla, PathBuilder, Pixels, Point, TextRun, Window, point, px, rgb};
use theme::ActiveTheme;

/// The resolved role palette for a viz scene.
///
/// Structure and status colors come from the active theme; the asset and
/// protocol accents (socket, giftwrap, bitcoin, lightning, liquid) mirror the
/// Bazaar design tokens and have this single definition point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VizPalette {
    pub node: Hsla,
    pub node_fill: Hsla,
    pub node_text: Hsla,
    pub muted: Hsla,
    pub socket: Hsla,
    pub giftwrap: Hsla,
    pub channel: Hsla,
    pub bitcoin: Hsla,
    pub liquid: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub danger: Hsla,
    pub boundary: Hsla,
}

impl VizPalette {
    pub fn from_theme(cx: &App) -> Self {
        let colors = cx.theme().colors();
        let status = cx.theme().status();
        Self {
            node: colors.border,
            node_fill: colors.surface_background,
            node_text: colors.text,
            muted: colors.text_muted,
            socket: status.info,
            giftwrap: rgb(0xa5b4fc).into(),
            channel: rgb(0xe8cb2b).into(),
            bitcoin: rgb(0xf7931a).into(),
            liquid: rgb(0x009bc0).into(),
            ok: status.success,
            warn: status.warning,
            danger: status.error,
            boundary: colors.border,
        }
    }

    /// Desaturates every role — the audit that proves state and class stay
    /// legible through dash patterns and glyphs when hue is gone.
    pub fn grayscale(self) -> Self {
        Self {
            node: self.node.grayscale(),
            node_fill: self.node_fill.grayscale(),
            node_text: self.node_text.grayscale(),
            muted: self.muted.grayscale(),
            socket: self.socket.grayscale(),
            giftwrap: self.giftwrap.grayscale(),
            channel: self.channel.grayscale(),
            bitcoin: self.bitcoin.grayscale(),
            liquid: self.liquid.grayscale(),
            ok: self.ok.grayscale(),
            warn: self.warn.grayscale(),
            danger: self.danger.grayscale(),
            boundary: self.boundary.grayscale(),
        }
    }
}

/// The monospace font every in-scene kind number, pubkey, height, and amount
/// renders in.
pub(crate) fn viz_font(cx: &App) -> Font {
    theme::theme_settings(cx).buffer_font(cx).clone()
}

pub(crate) fn stroke_line(
    window: &mut Window,
    from: Point<Pixels>,
    to: Point<Pixels>,
    width: Pixels,
    dash: Option<(Pixels, Pixels)>,
    color: Hsla,
) {
    let mut builder = PathBuilder::stroke(width);
    if let Some((dash, gap)) = dash {
        builder = builder.dash_array(&[dash, gap]);
    }
    builder.move_to(from);
    builder.line_to(to);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn stroke_circle(
    window: &mut Window,
    center: Point<Pixels>,
    radius: Pixels,
    width: Pixels,
    dash: Option<(Pixels, Pixels)>,
    color: Hsla,
) {
    let mut builder = PathBuilder::stroke(width);
    if let Some((dash, gap)) = dash {
        builder = builder.dash_array(&[dash, gap]);
    }
    builder.move_to(point(center.x + radius, center.y));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(center.x - radius, center.y),
    );
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(center.x + radius, center.y),
    );
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn fill_circle(window: &mut Window, center: Point<Pixels>, radius: Pixels, color: Hsla) {
    let mut builder = PathBuilder::fill();
    builder.move_to(point(center.x + radius, center.y));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(center.x - radius, center.y),
    );
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(center.x + radius, center.y),
    );
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn stroke_rounded_rect(
    window: &mut Window,
    origin: Point<Pixels>,
    size: gpui::Size<Pixels>,
    corner_radius: Pixels,
    width: Pixels,
    dash: Option<(Pixels, Pixels)>,
    color: Hsla,
) {
    let radius = corner_radius.min(size.width / 2.).min(size.height / 2.);
    let (x0, y0) = (origin.x, origin.y);
    let (x1, y1) = (origin.x + size.width, origin.y + size.height);
    let mut builder = PathBuilder::stroke(width);
    if let Some((dash, gap)) = dash {
        builder = builder.dash_array(&[dash, gap]);
    }
    builder.move_to(point(x0 + radius, y0));
    builder.line_to(point(x1 - radius, y0));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(x1, y0 + radius),
    );
    builder.line_to(point(x1, y1 - radius));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(x1 - radius, y1),
    );
    builder.line_to(point(x0 + radius, y1));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(x0, y1 - radius),
    );
    builder.line_to(point(x0, y0 + radius));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(x0 + radius, y0),
    );
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// One shaped line of scene text built from colored runs, painted at a
/// computed origin so scenes can center or right-align without layout.
pub(crate) struct SceneText {
    text: String,
    runs: Vec<TextRun>,
    font_size: Pixels,
}

pub(crate) enum SceneTextAnchor {
    Center,
    Left,
    Right,
}

impl SceneText {
    pub(crate) fn new(font_size: Pixels) -> Self {
        Self {
            text: String::new(),
            runs: Vec::new(),
            font_size,
        }
    }

    pub(crate) fn push(&mut self, text: &str, font: Font, color: Hsla) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        self.runs.push(TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }

    /// Paints the line with `position` interpreted per `anchor`; `y` is the
    /// vertical center of the line.
    pub(crate) fn paint(
        self,
        window: &mut Window,
        cx: &mut App,
        anchor: SceneTextAnchor,
        position: Point<Pixels>,
    ) {
        if self.text.is_empty() {
            return;
        }
        let line =
            window
                .text_system()
                .shape_line(self.text.into(), self.font_size, &self.runs, None);
        let line_height = self.font_size * 1.4;
        let x = match anchor {
            SceneTextAnchor::Center => position.x - line.width / 2.,
            SceneTextAnchor::Left => position.x,
            SceneTextAnchor::Right => position.x - line.width,
        };
        let origin = point(x, position.y - line_height / 2.);
        if line
            .paint(origin, line_height, gpui::TextAlign::Left, None, window, cx)
            .is_err()
        {
            log::warn!("viz scene text failed to paint");
        }
    }
}
