//! OpenAgents chat surface rendered with GPUI, compiled to WebAssembly.
//!
//! This is a demo of the chat components (thread rail, message list, agent
//! activity rows, executor disclosure, composer) running in a browser tab
//! through `gpui_web` + `gpui_wgpu` → WebGPU. No DOM, no React: every pixel
//! here is GPUI.

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, ElementId, SharedString, Task, Window, WindowBounds, WindowOptions, div,
    px, rgb, size,
};
use gpui::{Font, Pixels};
use theme::{ActiveTheme, ThemeSettingsProvider, UiDensity};
use ui::{
    Button, ButtonCommon, ButtonSize, ButtonStyle, Chip, Clickable, Color, Disableable, IconName,
    Indicator, Label, LabelCommon, LabelSize,
};

// ---------------------------------------------------------------------------
// Palette — Omega-ish dark surface
// ---------------------------------------------------------------------------

const BG_BASE: u32 = 0x0f1014;
const BG_RAIL: u32 = 0x14161c;
const BG_SURFACE: u32 = 0x1a1d25;
const BG_ELEVATED: u32 = 0x23262f;
const BORDER: u32 = 0x2b2f3a;
const TEXT: u32 = 0xe6e8ee;
const TEXT_DIM: u32 = 0x8b90a0;
const TEXT_FAINT: u32 = 0x5c6070;
const ACCENT: u32 = 0x7dd3fc;
const ACCENT_WARM: u32 = 0xfbbf24;
const ACCENT_GREEN: u32 = 0x86efac;
const USER_BUBBLE: u32 = 0x1e3a4f;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Agent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivityKind {
    Action,
    Artifact,
    Verification,
}

impl ActivityKind {
    fn label(self) -> &'static str {
        match self {
            ActivityKind::Action => "ACTION",
            ActivityKind::Artifact => "ARTIFACT",
            ActivityKind::Verification => "VERIFICATION",
        }
    }

    fn color(self) -> u32 {
        match self {
            ActivityKind::Action => ACCENT,
            ActivityKind::Artifact => ACCENT_WARM,
            ActivityKind::Verification => ACCENT_GREEN,
        }
    }

    /// Semantic color from the real Omega palette rather than a hard-coded hex.
    fn chip_color(self) -> Color {
        match self {
            ActivityKind::Action => Color::Accent,
            ActivityKind::Artifact => Color::Warning,
            ActivityKind::Verification => Color::Success,
        }
    }

    fn icon(self) -> IconName {
        match self {
            ActivityKind::Action => IconName::BoltFilled,
            ActivityKind::Artifact => IconName::FileCode,
            ActivityKind::Verification => IconName::Check,
        }
    }
}

#[derive(Clone)]
enum Entry {
    Message {
        role: Role,
        body: SharedString,
    },
    Activity {
        kind: ActivityKind,
        summary: SharedString,
        detail: SharedString,
    },
}

struct Thread {
    title: SharedString,
    subtitle: SharedString,
    unread: bool,
    entries: Vec<Entry>,
}

fn seeded_threads() -> Vec<Thread> {
    vec![
        Thread {
            title: "Coldcard entropy scan".into(),
            subtitle: "arm B · submodules materialized".into(),
            unread: false,
            entries: vec![
                Entry::Message {
                    role: Role::User,
                    body: "Run the entropy profile against the pre-fix commit. \
                           Materialize submodules this time."
                        .into(),
                },
                Entry::Activity {
                    kind: ActivityKind::Action,
                    summary: "git submodule update --init --recursive".into(),
                    detail: "2 submodules populated · 458 files now readable".into(),
                },
                Entry::Message {
                    role: Role::Agent,
                    body: "Source set materialized. Arm A saw 16 files; this tree has 458. \
                           Starting the ranked entropy sweep from seed generation outward."
                        .into(),
                },
                Entry::Activity {
                    kind: ActivityKind::Artifact,
                    summary: "coverage attestation published".into(),
                    detail: "completeness: complete · profile btc-firmware@1.2".into(),
                },
                Entry::Message {
                    role: Role::Agent,
                    body: "Finding: mpconfigboard.h:77 — the #ifndef guard tests existence, \
                           not value, so the disabled-RNG branch can win the link and seed \
                           generation reaches a deterministic PRNG. CWE-338."
                        .into(),
                },
                Entry::Activity {
                    kind: ActivityKind::Verification,
                    summary: "independent verdict: confirmed".into(),
                    detail: "verifier key ≠ producer key · evidence digest bound".into(),
                },
            ],
        },
        Thread {
            title: "BDK descriptor parser".into(),
            subtitle: "never examined".into(),
            unread: true,
            entries: vec![Entry::Message {
                role: Role::Agent,
                body: "No coverage attestation exists for this target at any commit.".into(),
            }],
        },
        Thread {
            title: "rust-lightning nonces".into(),
            subtitle: "watch · 3 revisions behind".into(),
            unread: false,
            entries: vec![Entry::Message {
                role: Role::Agent,
                body: "Regression watch is stale. Last invariant evaluation was 3 revisions ago."
                    .into(),
            }],
        },
    ]
}

const SUGGESTIONS: [&str; 3] = [
    "Scan the next unexamined target",
    "Show divergence between arm A and arm B",
    "Draft the disclosure for this finding",
];

struct ChatDemo {
    threads: Vec<Thread>,
    selected: usize,
    pending: bool,
    tick: usize,
    _tasks: Vec<Task<()>>,
}

impl ChatDemo {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            threads: seeded_threads(),
            selected: 0,
            pending: false,
            tick: 0,
            _tasks: Vec::new(),
        }
    }

    fn send(&mut self, text: &'static str, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.threads[self.selected].entries.push(Entry::Message {
            role: Role::User,
            body: text.into(),
        });
        self.pending = true;
        self.tick = 0;
        cx.notify();

        // Simulated streaming reply: a background task per step, so this
        // exercises the real gpui_web dispatcher and worker path.
        let selected = self.selected;
        let task = cx.spawn(async move |this, cx| {
            for step in 0..3usize {
                let spun = cx
                    .background_spawn(async move {
                        let mut acc: u64 = 0;
                        for i in 0..2_000_000u64 {
                            acc = acc.wrapping_add(i ^ (step as u64));
                        }
                        acc
                    })
                    .await;
                let _ = spun;
                this.update(cx, |this, cx| {
                    this.tick = step + 1;
                    cx.notify();
                })
                .ok();
            }

            this.update(cx, |this, cx| {
                this.threads[selected].entries.push(Entry::Activity {
                    kind: ActivityKind::Action,
                    summary: "claim acquired".into(),
                    detail: "repository work claim · generation 1".into(),
                });
                this.threads[selected].entries.push(Entry::Message {
                    role: Role::Agent,
                    body: "Claimed the target and pinned the base commit. \
                           Coverage attestation will publish when the sweep completes."
                        .into(),
                });
                this.pending = false;
                cx.notify();
            })
            .ok();
        });
        self._tasks.push(task);
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl ChatDemo {
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let mut rail = div()
            .flex()
            .flex_col()
            .w(px(248.))
            .flex_shrink_0()
            .h_full()
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(colors.border)
            .child(
                div().px_4().py_3().child(
                    Label::new("HARDENING THREADS")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            );

        for (index, thread) in self.threads.iter().enumerate() {
            let is_selected = index == self.selected;
            rail = rail.child(
                div()
                    .id(ElementId::NamedInteger("thread".into(), index as u64))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .mx_2()
                    .mb_1()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(is_selected, |this| this.bg(colors.element_selected))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.selected = index;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                Label::new(thread.title.clone())
                                    .size(LabelSize::Small)
                                    .color(if is_selected {
                                        Color::Default
                                    } else {
                                        Color::Muted
                                    }),
                            )
                            .when(thread.unread, |this| {
                                this.child(Indicator::dot().color(Color::Warning))
                            }),
                    )
                    .child(
                        Label::new(thread.subtitle.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        rail
    }

    fn render_entry(
        &self,
        entry: &Entry,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.theme().colors();
        match entry {
            Entry::Message { role, body } => {
                let is_user = *role == Role::User;
                div()
                    .id(ElementId::NamedInteger("entry".into(), index as u64))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Label::new(if is_user { "YOU" } else { "AGENT · codex-3" })
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        div()
                            .max_w(px(560.))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(if is_user {
                                colors.element_selected
                            } else {
                                colors.elevated_surface_background
                            })
                            .border_1()
                            .border_color(colors.border)
                            .child(Label::new(body.clone()).size(LabelSize::Small)),
                    )
            }
            Entry::Activity {
                kind,
                summary,
                detail,
            } => div()
                .id(ElementId::NamedInteger("entry".into(), index as u64))
                .flex()
                .flex_col()
                .gap_1()
                .max_w(px(560.))
                .px_3()
                .py_2()
                .rounded_md()
                .bg(colors.surface_background)
                .border_1()
                .border_color(colors.border)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            Chip::new(kind.label())
                                .label_color(kind.chip_color())
                                .icon(kind.icon())
                                .icon_color(kind.chip_color()),
                        )
                        .child(Label::new(summary.clone()).size(LabelSize::Small)),
                )
                .child(
                    Label::new(detail.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        }
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let mut chips = div().flex().flex_row().flex_wrap().gap_2();
        for (index, suggestion) in SUGGESTIONS.iter().enumerate() {
            chips = chips.child(
                Button::new(
                    ElementId::NamedInteger("chip".into(), index as u64),
                    *suggestion,
                )
                .style(ButtonStyle::Outlined)
                .size(ButtonSize::Compact)
                .label_size(LabelSize::XSmall)
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.send(SUGGESTIONS[index], cx);
                })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_4()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(chips)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(colors.editor_background)
                    .border_1()
                    .border_color(colors.border)
                    .child(
                        div().flex_1().child(
                            Label::new(if self.pending {
                                "Agent is working…"
                            } else {
                                "Pick a suggestion above to send"
                            })
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                        ),
                    )
                    .child(
                        Button::new("send", if self.pending { "Working" } else { "Send" })
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .disabled(self.pending),
                    ),
            )
    }
}

impl Render for ChatDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread_title = self.threads[self.selected].title.clone();
        let entries = self.threads[self.selected].entries.clone();

        let mut transcript = div().flex().flex_col().gap_4().px_5().py_4();
        for (index, entry) in entries.iter().enumerate() {
            transcript = transcript.child(self.render_entry(entry, index, cx));
        }
        if self.pending {
            let dots = match self.tick {
                0 => "· reading source",
                1 => "· ranking attack surface",
                2 => "· pinning commit",
                _ => "· writing claim",
            };
            transcript = transcript.child(
                Label::new(format!("agent {dots}"))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        }

        let bg = cx.theme().colors().background;
        let border_color = cx.theme().colors().border;
        let elevated = cx.theme().colors().elevated_surface_background;
        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(bg)
            .child(self.render_rail(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .size_full()
                    // Header with executor disclosure — the thread names what ran.
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px_5()
                            .py_3()
                            .border_b_1()
                            .border_color(border_color)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(Label::new(thread_title))
                                    .child(
                                        Label::new("GPUI · WebGPU · wasm32-unknown-unknown")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(elevated)
                                    .border_1()
                                    .border_color(border_color)
                                    .child(Indicator::dot().color(Color::Success))
                                    .child(
                                        Label::new("executor: codex-3 · grant gen 3")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("transcript")
                            .flex_1()
                            .overflow_y_scroll()
                            .child(transcript),
                    )
                    .child(self.render_composer(cx)),
            )
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// The `theme_settings` crate installs this global on native, but it depends on
/// `settings`, which does not build for wasm (native fs/file-watching). The
/// trait is only five methods, so the browser build supplies its own using the
/// fonts `gpui_web` already embeds (IBM Plex Sans + Lilex).
struct WebThemeSettings {
    ui_font: Font,
    buffer_font: Font,
}

impl ThemeSettingsProvider for WebThemeSettings {
    fn ui_font<'a>(&'a self, _cx: &'a App) -> &'a Font {
        &self.ui_font
    }

    fn buffer_font<'a>(&'a self, _cx: &'a App) -> &'a Font {
        &self.buffer_font
    }

    fn ui_font_size(&self, _cx: &App) -> Pixels {
        px(14.)
    }

    fn buffer_font_size(&self, _cx: &App) -> Pixels {
        px(13.)
    }

    fn ui_density(&self, _cx: &App) -> UiDensity {
        UiDensity::Default
    }
}

/// Omega's own "Aiur" theme, embedded. `theme::init` normally discovers this
/// through an AssetSource + `theme_settings::load_bundled_themes`, but
/// `theme_settings` does not build for wasm, so the browser build parses the
/// same checked-in JSON and patches the loaded theme's colors directly.
const AIUR_JSON: &str = include_str!("../../../../assets/themes/aiur/aiur.json");

fn hex_to_hsla(hex: &str) -> Option<gpui::Hsla> {
    let raw = hex.trim_start_matches('#');
    let (r, g, b, a) = match raw.len() {
        8 => (
            u8::from_str_radix(&raw[0..2], 16).ok()?,
            u8::from_str_radix(&raw[2..4], 16).ok()?,
            u8::from_str_radix(&raw[4..6], 16).ok()?,
            u8::from_str_radix(&raw[6..8], 16).ok()?,
        ),
        6 => (
            u8::from_str_radix(&raw[0..2], 16).ok()?,
            u8::from_str_radix(&raw[2..4], 16).ok()?,
            u8::from_str_radix(&raw[4..6], 16).ok()?,
            255,
        ),
        _ => return None,
    };
    Some(gpui::Hsla::from(gpui::Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: a as f32 / 255.,
    }))
}

/// Overwrite the active theme's colors with Aiur's, keyed by the same dotted
/// names the theme JSON uses.
fn apply_aiur_theme(cx: &mut App) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(AIUR_JSON) else {
        return;
    };
    let Some(style) = parsed
        .get("themes")
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("style"))
        .and_then(|s| s.as_object())
    else {
        return;
    };

    let get = |key: &str| -> Option<gpui::Hsla> {
        style
            .get(key)
            .and_then(|v| v.as_str())
            .and_then(hex_to_hsla)
    };

    let current = <App as theme::ActiveTheme>::theme(cx).clone();
    let mut next = (*current).clone();
    next.name = "Aiur".into();
    let c = &mut next.styles.colors;

    macro_rules! set {
        ($field:ident, $key:expr) => {
            if let Some(color) = get($key) {
                c.$field = color;
            }
        };
    }

    set!(background, "background");
    set!(surface_background, "surface.background");
    set!(elevated_surface_background, "elevated_surface.background");
    set!(panel_background, "panel.background");
    set!(editor_background, "editor.background");
    set!(element_background, "element.background");
    set!(element_hover, "element.hover");
    set!(element_active, "element.active");
    set!(element_selected, "element.selected");
    set!(ghost_element_hover, "ghost_element.hover");
    set!(ghost_element_selected, "ghost_element.selected");
    set!(border, "border");
    set!(border_variant, "border.variant");
    set!(border_focused, "border.focused");
    set!(text, "text");
    set!(text_muted, "text.muted");
    set!(text_placeholder, "text.placeholder");
    set!(text_disabled, "text.disabled");
    set!(text_accent, "text.accent");
    set!(icon, "icon");
    set!(icon_muted, "icon.muted");
    set!(icon_accent, "icon.accent");
    set!(status_bar_background, "status_bar.background");
    set!(title_bar_background, "title_bar.background");
    set!(toolbar_background, "toolbar.background");
    set!(tab_bar_background, "tab_bar.background");
    set!(tab_active_background, "tab.active_background");
    set!(tab_inactive_background, "tab.inactive_background");

    theme::GlobalTheme::update_theme(cx, std::sync::Arc::new(next));
}

fn main() {
    gpui_platform::web_init();

    // Real Omega theme. `LoadThemes::JustBase` needs no AssetSource, which is
    // what makes the design system usable in a browser with no filesystem.

    // On the web the run loop belongs to the browser: `Platform::run` invokes the
    // launch callback and returns immediately. `run` would therefore drop the App
    // as soon as `main` returns ("app was released", blank canvas). `run_embedded`
    // hands back a handle that owns the app state, and leaking it keeps the app
    // alive for the lifetime of the page.
    let handle = gpui_platform::application().run_embedded(|cx: &mut App| {
        theme::set_theme_settings_provider(
            Box::new(WebThemeSettings {
                ui_font: gpui::font("IBM Plex Sans"),
                buffer_font: gpui::font("Lilex"),
            }),
            cx,
        );
        theme::init(theme::LoadThemes::JustBase, cx);
        apply_aiur_theme(cx);
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(ChatDemo::new),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
    std::mem::forget(handle);
}
