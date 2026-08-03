//! Hermetic control-crawl protocol: drawn implies working.
//!
//! OMEGA-DELTA-0187 / owner review item 17. A visible control must produce an
//! observable consequence when activated with pointer and with keyboard, unless
//! a registered exemption names why. Menu entries are activated individually so
//! a display-only row cannot hide behind a parent that only looks interactive.
//! Escape must dismiss every modal the crawl opens.
//!
//! Full GPUI semantic-tree coverage expands scene by scene through the
//! checked-in registry. This crate owns the protocol, the synthetic proving
//! scene, the mutation proof, registry load, the multi-sentence copy lint, and
//! the source scan that holds Omega-owned chrome to the concise-copy contract.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Repository-root-relative path of the crawl registry.
pub const REGISTRY_PATH: &str = "docs/omega/control-crawl-registry.json";

/// Repository-root-relative path of the multi-sentence copy allowlist.
pub const COPY_ALLOWLIST_PATH: &str = "docs/omega/control-crawl-copy-allowlist.json";

/// Schema id the registry document must carry.
pub const REGISTRY_SCHEMA: &str = "openagents.omega.control-crawl-registry.v1";

/// Schema id the copy allowlist must carry.
pub const COPY_ALLOWLIST_SCHEMA: &str = "openagents.omega.control-crawl-copy-allowlist.v1";

/// How a control was activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationMethod {
    /// Pointer / click path.
    Pointer,
    /// Keyboard path (Enter/Space equivalent for the control).
    Keyboard,
}

/// Kind of interactive control the crawl knows how to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// Ordinary button or toggle.
    Button,
    /// Individual menu entry (must be activated on its own).
    MenuEntry,
    /// Control that opens a modal the crawl must later Escape-dismiss.
    OpensModal,
}

/// A registered reason a control may produce no observable consequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exemption {
    /// Human-readable reason; empty reasons are refused.
    pub reason: String,
}

impl Exemption {
    /// Build an exemption; panics in tests if the reason is empty via validate.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// Whether this exemption is well-formed.
    pub fn is_valid(&self) -> bool {
        !self.reason.trim().is_empty()
    }
}

/// One interactive control the crawl can activate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractiveControl {
    /// Stable id inside the scene.
    pub id: String,
    /// User-visible label (for failure messages).
    pub label: String,
    /// Control kind.
    pub kind: ControlKind,
    /// Optional exemption for zero-consequence activation.
    pub exemption: Option<Exemption>,
}

/// Observable snapshot used to detect consequence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneSnapshot {
    /// Opaque key/value observations the scene chooses to expose.
    pub observations: BTreeMap<String, String>,
}

impl SceneSnapshot {
    /// Insert one observation.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.observations.insert(key.into(), value.into());
    }
}

/// Outcome of one activation attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationOutcome {
    /// Snapshot after the activation.
    pub after: SceneSnapshot,
    /// Whether the control claimed to handle the activation.
    pub handled: bool,
}

/// A modal currently open in the scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenModal {
    /// Stable modal id.
    pub id: String,
    /// User-visible title.
    pub title: String,
}

/// Failure recorded by a crawl pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrawlFailure {
    /// Control or modal id.
    pub subject: String,
    /// Activation method when relevant.
    pub method: Option<ActivationMethod>,
    /// Why the crawl failed.
    pub detail: String,
}

/// Full report for one scene crawl.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrawlReport {
    /// Scene name.
    pub scene: String,
    /// Failures; empty means pass.
    pub failures: Vec<CrawlFailure>,
    /// Controls visited.
    pub controls_activated: usize,
    /// Modals escape-dismissed.
    pub modals_dismissed: usize,
}

impl CrawlReport {
    /// Whether the crawl passed.
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Scene contract the crawl drives. Production GPUI adapters implement this;
/// the synthetic proving scene is the first complete implementation.
pub trait CrawlScene {
    /// Stable scene name (must match a registry surface id when registered).
    fn name(&self) -> &str;

    /// Enumerate interactive controls currently reachable.
    fn enumerate_controls(&self) -> Vec<InteractiveControl>;

    /// Snapshot of observable state before/after activation.
    fn snapshot(&self) -> SceneSnapshot;

    /// Activate one control. Must not panic on unknown ids; return handled=false.
    fn activate(&mut self, control_id: &str, method: ActivationMethod) -> ActivationOutcome;

    /// Modals currently open (opened by prior activations in this crawl).
    fn open_modals(&self) -> Vec<OpenModal>;

    /// Dismiss a modal with Escape. Returns true when the modal is gone.
    fn dismiss_with_escape(&mut self, modal_id: &str) -> bool;
}

/// Crawl one scene: activate every control with pointer and keyboard, require
/// consequence unless exempted, and Escape-dismiss every modal opened.
pub fn crawl_scene(scene: &mut dyn CrawlScene) -> CrawlReport {
    let mut report = CrawlReport {
        scene: scene.name().to_string(),
        ..CrawlReport::default()
    };

    let controls = scene.enumerate_controls();
    for control in &controls {
        if let Some(exemption) = &control.exemption {
            if !exemption.is_valid() {
                report.failures.push(CrawlFailure {
                    subject: control.id.clone(),
                    method: None,
                    detail: "exemption reason is empty; name why this control may be inert"
                        .to_string(),
                });
                continue;
            }
        }

        for method in [ActivationMethod::Pointer, ActivationMethod::Keyboard] {
            let before = scene.snapshot();
            let outcome = scene.activate(&control.id, method);
            report.controls_activated += 1;

            if !outcome.handled {
                report.failures.push(CrawlFailure {
                    subject: control.id.clone(),
                    method: Some(method),
                    detail: format!(
                        "control {:?} ({}) did not handle {:?} activation",
                        control.label, control.id, method
                    ),
                });
                continue;
            }

            let changed = outcome.after != before;
            if !changed {
                match &control.exemption {
                    Some(exemption) if exemption.is_valid() => {}
                    _ => {
                        report.failures.push(CrawlFailure {
                            subject: control.id.clone(),
                            method: Some(method),
                            detail: format!(
                                "zero observable consequence for {:?} ({}) via {:?}; \
                                 drawn implies working unless a registered exemption names a reason",
                                control.label, control.id, method
                            ),
                        });
                    }
                }
            }
        }
    }

    // Capture modal ids first so we can dismiss without holding a borrow.
    let modal_ids: Vec<(String, String)> = scene
        .open_modals()
        .into_iter()
        .map(|modal| (modal.id, modal.title))
        .collect();
    for (modal_id, title) in modal_ids {
        if scene.dismiss_with_escape(&modal_id) {
            report.modals_dismissed += 1;
            // Confirm it is actually gone.
            let still_open = scene
                .open_modals()
                .into_iter()
                .any(|modal| modal.id == modal_id);
            if still_open {
                report.failures.push(CrawlFailure {
                    subject: modal_id,
                    method: None,
                    detail: format!("modal {title:?} claimed Escape dismissal but is still open"),
                });
            }
        } else {
            report.failures.push(CrawlFailure {
                subject: modal_id,
                method: None,
                detail: format!("modal {title:?} did not dismiss on Escape; every modal must"),
            });
        }
    }

    report
}

// ---------------------------------------------------------------------------
// Synthetic proving scene
// ---------------------------------------------------------------------------

/// Core proving scene used by cargo tests and the release-gate row.
///
/// Contains a working toggle, a working menu entry, a modal opener with Escape
/// dismissal, and — when `inject_noop` is true — a deliberate no-op control
/// that the mutation-proof test expects to fail the crawl.
#[derive(Clone, Debug)]
pub struct ProvingScene {
    /// When true, include a deliberate no-op control the crawl must fail on.
    pub inject_noop: bool,
    toggle_on: bool,
    menu_fired: u32,
    modal_open: bool,
    modal_opened_count: u32,
}

impl Default for ProvingScene {
    fn default() -> Self {
        Self::new(false)
    }
}

impl ProvingScene {
    /// Build a proving scene. Pass `inject_noop = true` only from the mutation
    /// proof test.
    pub fn new(inject_noop: bool) -> Self {
        Self {
            inject_noop,
            toggle_on: false,
            menu_fired: 0,
            modal_open: false,
            modal_opened_count: 0,
        }
    }
}

impl CrawlScene for ProvingScene {
    fn name(&self) -> &str {
        "proving-synthetic"
    }

    fn enumerate_controls(&self) -> Vec<InteractiveControl> {
        let mut controls = vec![
            InteractiveControl {
                id: "toggle-working".into(),
                label: "Working toggle".into(),
                kind: ControlKind::Button,
                exemption: None,
            },
            InteractiveControl {
                id: "menu-entry-working".into(),
                label: "Working menu entry".into(),
                kind: ControlKind::MenuEntry,
                exemption: None,
            },
            InteractiveControl {
                id: "open-modal".into(),
                label: "Open modal".into(),
                kind: ControlKind::OpensModal,
                exemption: None,
            },
        ];
        if self.inject_noop {
            controls.push(InteractiveControl {
                id: "deliberate-noop".into(),
                label: "Deliberate no-op".into(),
                kind: ControlKind::Button,
                exemption: None,
            });
        }
        controls
    }

    fn snapshot(&self) -> SceneSnapshot {
        let mut snapshot = SceneSnapshot::default();
        snapshot.insert("toggle_on", self.toggle_on.to_string());
        snapshot.insert("menu_fired", self.menu_fired.to_string());
        snapshot.insert("modal_open", self.modal_open.to_string());
        snapshot.insert("modal_opened_count", self.modal_opened_count.to_string());
        snapshot
    }

    fn activate(&mut self, control_id: &str, _method: ActivationMethod) -> ActivationOutcome {
        match control_id {
            "toggle-working" => {
                self.toggle_on = !self.toggle_on;
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: true,
                }
            }
            "menu-entry-working" => {
                self.menu_fired = self.menu_fired.saturating_add(1);
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: true,
                }
            }
            "open-modal" => {
                // Every activation is observable: bump the open count even when
                // the modal is already up so pointer then keyboard both change
                // the snapshot (a real opener re-asserts focus on second arm).
                self.modal_open = true;
                self.modal_opened_count = self.modal_opened_count.saturating_add(1);
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: true,
                }
            }
            "deliberate-noop" => {
                // Handled but inert: the ContextMenuEntry.action trap shape.
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: true,
                }
            }
            _ => ActivationOutcome {
                after: self.snapshot(),
                handled: false,
            },
        }
    }

    fn open_modals(&self) -> Vec<OpenModal> {
        if self.modal_open {
            vec![OpenModal {
                id: "proving-modal".into(),
                title: "Proving modal".into(),
            }]
        } else {
            Vec::new()
        }
    }

    fn dismiss_with_escape(&mut self, modal_id: &str) -> bool {
        if modal_id == "proving-modal" && self.modal_open {
            self.modal_open = false;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Surface status in the crawl registry.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceStatus {
    /// Crawl coverage is implemented and enforced.
    Complete,
    /// Surface is known; crawl implementation still open.
    PendingExpansion,
    /// Explicitly out of crawl with a reason on the entry or exemptions list.
    Exempt,
}

/// One surface row in the crawl registry.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RegistrySurface {
    /// Stable surface id.
    pub id: String,
    /// Kind tag (synthetic, hermetic-scene, modal, menu, …).
    pub kind: String,
    /// Coverage status.
    pub status: SurfaceStatus,
    /// Optional owning crate.
    #[serde(default)]
    pub crate_name: Option<String>,
    /// Description.
    #[serde(default)]
    pub description: String,
}

/// Checked-in crawl registry document.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CrawlRegistry {
    /// Schema id.
    pub schema: String,
    /// Surfaces the product must not forget.
    pub surfaces: Vec<RegistrySurface>,
}

impl CrawlRegistry {
    /// Parse registry JSON.
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| format!("control-crawl registry is not JSON: {error}"))?;
        // Accept either `crate` or `crate_name` in the document.
        let normalized = normalize_registry_json(value)?;
        let registry: Self = serde_json::from_value(normalized)
            .map_err(|error| format!("control-crawl registry shape is wrong: {error}"))?;
        if registry.schema != REGISTRY_SCHEMA {
            return Err(format!(
                "control-crawl registry schema is {:?}, expected {REGISTRY_SCHEMA}",
                registry.schema
            ));
        }
        if registry.surfaces.is_empty() {
            return Err("control-crawl registry has no surfaces".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for surface in &registry.surfaces {
            if surface.id.trim().is_empty() {
                return Err("control-crawl registry has a surface with an empty id".into());
            }
            if !seen.insert(surface.id.clone()) {
                return Err(format!(
                    "control-crawl registry repeats surface id {:?}",
                    surface.id
                ));
            }
        }
        Ok(registry)
    }

    /// Load the checked-in registry from the repository root.
    pub fn load_from_repository() -> Result<Self, String> {
        let path = repository_path(REGISTRY_PATH);
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        Self::parse(&raw)
    }

    /// Whether a surface id is registered.
    pub fn contains(&self, id: &str) -> bool {
        self.surfaces.iter().any(|surface| surface.id == id)
    }

    /// Surface ids with complete crawl coverage.
    pub fn complete_ids(&self) -> Vec<&str> {
        self.surfaces
            .iter()
            .filter(|surface| surface.status == SurfaceStatus::Complete)
            .map(|surface| surface.id.as_str())
            .collect()
    }
}

fn normalize_registry_json(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(surfaces) = value
        .get_mut("surfaces")
        .and_then(|item| item.as_array_mut())
    else {
        return Ok(value);
    };
    for surface in surfaces {
        let Some(object) = surface.as_object_mut() else {
            continue;
        };
        if let Some(crate_value) = object.remove("crate") {
            object.insert("crate_name".into(), crate_value);
        }
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Copy lint (owner law 2)
// ---------------------------------------------------------------------------

/// One allowlisted multi-sentence string.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CopyAllowlistEntry {
    /// Exact string allowed.
    pub text: String,
    /// Why it is allowed.
    pub reason: String,
}

/// Copy allowlist document.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CopyAllowlist {
    /// Schema id.
    pub schema: String,
    /// Exact allowed strings.
    #[serde(default)]
    pub entries: Vec<CopyAllowlistEntry>,
}

impl CopyAllowlist {
    /// Parse allowlist JSON.
    pub fn parse(json: &str) -> Result<Self, String> {
        let allowlist: Self = serde_json::from_str(json)
            .map_err(|error| format!("copy allowlist is not valid JSON: {error}"))?;
        if allowlist.schema != COPY_ALLOWLIST_SCHEMA {
            return Err(format!(
                "copy allowlist schema is {:?}, expected {COPY_ALLOWLIST_SCHEMA}",
                allowlist.schema
            ));
        }
        for entry in &allowlist.entries {
            if entry.text.trim().is_empty() {
                return Err("copy allowlist has an entry with empty text".into());
            }
            if entry.reason.trim().is_empty() {
                return Err(format!(
                    "copy allowlist entry for {:?} has an empty reason",
                    entry.text
                ));
            }
        }
        Ok(allowlist)
    }

    /// Load the checked-in allowlist from the repository root.
    pub fn load_from_repository() -> Result<Self, String> {
        let path = repository_path(COPY_ALLOWLIST_PATH);
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        Self::parse(&raw)
    }

    /// Whether `text` is allowlisted.
    pub fn allows(&self, text: &str) -> bool {
        self.entries.iter().any(|entry| entry.text == text)
    }
}

/// Whether `text` looks like multi-sentence exposition (owner law 2).
///
/// A string is multi-sentence when it contains a sentence terminator (`.`,
/// `!`, or `?`) followed by whitespace and a further alphabetic character.
/// Single short phrases and one-word tooltips pass.
pub fn is_multi_sentence(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let bytes = trimmed.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        let ch = bytes[index];
        if matches!(ch, b'.' | b'!' | b'?') {
            let mut look = index + 1;
            while look < bytes.len() && bytes[look].is_ascii_whitespace() {
                look += 1;
            }
            if look < bytes.len() && bytes[look].is_ascii_alphabetic() {
                return true;
            }
        }
        index += 1;
    }
    false
}

/// Lint a set of user-facing strings; returns each multi-sentence string that
/// is not on the allowlist.
pub fn lint_copy<'a>(
    strings: impl IntoIterator<Item = &'a str>,
    allowlist: &CopyAllowlist,
) -> Vec<String> {
    let mut offenders = Vec::new();
    for text in strings {
        if is_multi_sentence(text) && !allowlist.allows(text) {
            offenders.push(text.to_string());
        }
    }
    offenders.sort();
    offenders.dedup();
    offenders
}

/// Status vocabulary admitted for one-word tooltips and accessibility cues.
pub const REGISTERED_STATUS_WORDS: &[&str] = &[
    "Blocked", "Complete", "Failed", "Offline", "Ready", "Running", "Warning",
];

/// Lint strings declared as presentation statuses. Multi-word and unknown
/// labels fail; detailed domain facts belong outside this status channel.
pub fn lint_status_words<'a>(strings: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut offenders = strings
        .into_iter()
        .filter(|value| {
            value.split_whitespace().count() != 1 || !REGISTERED_STATUS_WORDS.contains(value)
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    offenders.sort();
    offenders.dedup();
    offenders
}

// ---------------------------------------------------------------------------
// Omega-owned presentation copy inventory
// ---------------------------------------------------------------------------

/// Where a user-facing string literal was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PresentationSlot {
    /// `.aria_label("...")` — the accessible name.
    AccessibleName,
    /// `Tooltip::text("...")` — hover/focus text.
    Tooltip,
    /// `.child("...")` — a visible label rendered directly as chrome.
    VisibleLabel,
}

impl PresentationSlot {
    const fn marker(self) -> &'static str {
        match self {
            Self::AccessibleName => ".aria_label(",
            Self::Tooltip => "Tooltip::text(",
            Self::VisibleLabel => ".child(",
        }
    }

    /// Longest string this slot may hold before it is exposition rather than a
    /// label. The accessible name is deliberately the most generous limit:
    /// conciseness must never truncate assistive meaning. A tooltip may name a
    /// second gesture for the same control, so it shares the label limit; the
    /// one-word rule belongs to the status channel and is enforced separately
    /// by `lint_status_words`.
    pub const fn max_chars(self) -> usize {
        match self {
            Self::AccessibleName => 80,
            Self::Tooltip | Self::VisibleLabel => 48,
        }
    }

    const fn describe(self) -> &'static str {
        match self {
            Self::AccessibleName => "accessible name",
            Self::Tooltip => "tooltip",
            Self::VisibleLabel => "visible label",
        }
    }
}

/// One user-facing string literal found in Omega-owned source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PresentationString {
    /// Repository-relative source file.
    pub file: String,
    /// Where the literal was written.
    pub slot: PresentationSlot,
    /// The decoded literal.
    pub text: String,
}

/// A copy-contract violation the lint refuses.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CopyOffense {
    /// The offending string.
    pub string: PresentationString,
    /// Why it is refused.
    pub detail: String,
}

/// Extract user-facing string literals from one Rust source file.
///
/// Only literal arguments are visible to a source scan; a `format!` or a
/// source-owned domain record is out of reach and is governed by the surface's
/// own tests instead. The `#[cfg(test)]` module is excluded because test
/// fixtures are not shipped chrome.
#[must_use]
pub fn scan_presentation_copy(file: &str, source: &str) -> Vec<PresentationString> {
    let shipped = &source[..shipped_source_length(source)];
    let mut found = Vec::new();
    for slot in [
        PresentationSlot::AccessibleName,
        PresentationSlot::Tooltip,
        PresentationSlot::VisibleLabel,
    ] {
        let marker = slot.marker();
        let mut search = 0;
        while let Some(offset) = shipped[search..].find(marker) {
            let after = search + offset + marker.len();
            search = after;
            let rest = shipped[after..].trim_start();
            if !rest.starts_with('"') {
                continue;
            }
            let Some(text) = decode_rust_string_literal(&rest[1..]) else {
                continue;
            };
            found.push(PresentationString {
                file: file.to_string(),
                slot,
                text,
            });
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Length of the shipped prefix of a source file, excluding a trailing
/// `#[cfg(test)] mod ... { }` module.
///
/// A file may also gate individual items on `#[cfg(test)]`, so cutting at the
/// first occurrence would silently stop scanning most of the file. The cut is
/// taken at the last top-level attribute that introduces a module, which is the
/// conventional position of the test module.
fn shipped_source_length(source: &str) -> usize {
    const ATTRIBUTE: &str = "\n#[cfg(test)]";
    let mut cut = source.len();
    let mut search = 0;
    while let Some(offset) = source[search..].find(ATTRIBUTE) {
        let index = search + offset;
        search = index + ATTRIBUTE.len();
        let introduces_module = source[search..]
            .lines()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| {
                let line = line.trim_start();
                line.starts_with("mod ")
                    || line.starts_with("pub mod ")
                    || line.starts_with("pub(crate) mod ")
            });
        if introduces_module {
            cut = index;
        }
    }
    cut
}

/// Decode a Rust string literal body that starts immediately after the opening
/// quote. Returns `None` when the literal is unterminated.
fn decode_rust_string_literal(body: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => return Some(decoded),
            '\\' => {
                let escaped = characters.next()?;
                decoded.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                });
            }
            other => decoded.push(other),
        }
    }
    None
}

/// This lint's own source quotes the call markers it searches for, so scanning
/// it would report its own definitions. It renders no chrome.
const LINT_SOURCE: &str = "crates/omega_control_crawl/src/omega_control_crawl.rs";

/// Repository-relative Omega-owned UI source files the copy contract governs.
///
/// Discovery is by location, not by an enumerated list, so a new Omega
/// destination is covered the moment it is added.
pub fn omega_owned_ui_sources() -> Result<Vec<String>, String> {
    let mut sources = Vec::new();
    let crates_root = repository_path("crates");
    let entries = std::fs::read_dir(&crates_root)
        .map_err(|error| format!("cannot read {}: {error}", crates_root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a crates entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("omega_") {
            continue;
        }
        collect_rust_sources(
            &entry.path().join("src"),
            &format!("crates/{name}/src"),
            &mut sources,
        )?;
    }
    let agent_ui_src = repository_path("crates/agent_ui/src");
    let mut agent_ui_sources = Vec::new();
    collect_rust_sources(&agent_ui_src, "crates/agent_ui/src", &mut agent_ui_sources)?;
    sources.extend(agent_ui_sources.into_iter().filter(|path| {
        let name = path.rsplit('/').next().unwrap_or_default();
        name.starts_with("omega_")
            || matches!(
                name,
                "agent_panel.rs"
                    | "forensics_workbench.rs"
                    | "effective_principal.rs"
                    | "organization_scope.rs"
            )
    }));
    sources.retain(|source| source != LINT_SOURCE);
    sources.sort();
    sources.dedup();
    if sources.is_empty() {
        return Err("no Omega-owned UI sources were discovered".into());
    }
    Ok(sources)
}

fn collect_rust_sources(
    directory: &Path,
    relative: &str,
    sources: &mut Vec<String>,
) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {relative}: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let child_relative = format!("{relative}/{name}");
        if path.is_dir() {
            collect_rust_sources(&path, &child_relative, sources)?;
        } else if name.ends_with(".rs") {
            sources.push(child_relative);
        }
    }
    Ok(())
}

/// Refuse multi-sentence chrome and overlong presentation strings.
///
/// Deleting a string is preferred over allowlisting it; the allowlist exists
/// for exact domain records that must survive verbatim.
pub fn lint_presentation_copy(
    strings: &[PresentationString],
    allowlist: &CopyAllowlist,
) -> Vec<CopyOffense> {
    let mut offenses = Vec::new();
    for string in strings {
        if allowlist.allows(&string.text) {
            continue;
        }
        if is_multi_sentence(&string.text) {
            offenses.push(CopyOffense {
                string: string.clone(),
                detail: format!("{} narrates more than one sentence", string.slot.describe()),
            });
            continue;
        }
        let length = string.text.chars().count();
        let limit = string.slot.max_chars();
        if length > limit {
            offenses.push(CopyOffense {
                string: string.clone(),
                detail: format!(
                    "{} is {length} characters; the limit is {limit}",
                    string.slot.describe()
                ),
            });
        }
    }
    offenses.sort();
    offenses.dedup();
    offenses
}

/// Scan and lint every Omega-owned UI source in the repository.
pub fn lint_repository_presentation_copy(
    allowlist: &CopyAllowlist,
) -> Result<Vec<CopyOffense>, String> {
    let mut strings = Vec::new();
    for file in omega_owned_ui_sources()? {
        let path = repository_path(&file);
        let source = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        strings.extend(scan_presentation_copy(&file, &source));
    }
    Ok(lint_presentation_copy(&strings, allowlist))
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Resolve a repository-root-relative path from this crate's manifest dir.
#[must_use]
pub fn repository_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proving_scene_crawl_passes() {
        let mut scene = ProvingScene::new(false);
        let report = crawl_scene(&mut scene);
        assert!(
            report.passed(),
            "proving scene must pass the crawl; failures: {:?}",
            report.failures
        );
        assert!(
            report.controls_activated >= 6,
            "expected pointer+keyboard on three controls, got {}",
            report.controls_activated
        );
        assert_eq!(
            report.modals_dismissed, 1,
            "open-modal must leave a modal that Escape dismisses"
        );
        assert!(
            !scene.modal_open,
            "Escape dismissal must leave the proving modal closed"
        );
    }

    /// Mutation proof: a deliberate no-op control must fail the crawl.
    ///
    /// If this test ever stops failing while `inject_noop` is true, the gate
    /// can no longer see inert controls and must not land green.
    #[test]
    fn deliberate_noop_control_fails_the_crawl() {
        let mut scene = ProvingScene::new(true);
        let report = crawl_scene(&mut scene);
        assert!(
            !report.passed(),
            "mutation proof inverted: a deliberate no-op passed the crawl"
        );
        let noop_failures: Vec<_> = report
            .failures
            .iter()
            .filter(|failure| failure.subject == "deliberate-noop")
            .collect();
        assert!(
            !noop_failures.is_empty(),
            "expected failures on deliberate-noop, got {:?}",
            report.failures
        );
        assert!(
            noop_failures
                .iter()
                .any(|failure| { failure.detail.contains("zero observable consequence") }),
            "noop failures must name zero observable consequence: {noop_failures:?}"
        );
        // Pointer and keyboard both required.
        let methods: Vec<_> = noop_failures
            .iter()
            .filter_map(|failure| failure.method)
            .collect();
        assert!(
            methods.contains(&ActivationMethod::Pointer)
                && methods.contains(&ActivationMethod::Keyboard),
            "noop must fail for both pointer and keyboard: {methods:?}"
        );
    }

    #[test]
    fn menu_entries_are_activated_individually() {
        let mut scene = ProvingScene::new(false);
        let before = scene.menu_fired;
        let report = crawl_scene(&mut scene);
        assert!(report.passed(), "{:?}", report.failures);
        // Pointer + keyboard each fire the menu entry once.
        assert_eq!(
            scene.menu_fired,
            before + 2,
            "menu entries must be activated individually on both input paths"
        );
    }

    #[test]
    fn escape_dismissal_is_required_for_open_modals() {
        struct StickyModal;
        impl CrawlScene for StickyModal {
            fn name(&self) -> &str {
                "sticky-modal"
            }
            fn enumerate_controls(&self) -> Vec<InteractiveControl> {
                Vec::new()
            }
            fn snapshot(&self) -> SceneSnapshot {
                SceneSnapshot::default()
            }
            fn activate(
                &mut self,
                _control_id: &str,
                _method: ActivationMethod,
            ) -> ActivationOutcome {
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: false,
                }
            }
            fn open_modals(&self) -> Vec<OpenModal> {
                vec![OpenModal {
                    id: "stuck".into(),
                    title: "Stuck".into(),
                }]
            }
            fn dismiss_with_escape(&mut self, _modal_id: &str) -> bool {
                false
            }
        }

        let mut scene = StickyModal;
        let report = crawl_scene(&mut scene);
        assert!(!report.passed());
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.detail.contains("did not dismiss on Escape")),
            "{:?}",
            report.failures
        );
    }

    #[test]
    fn valid_exemption_allows_inert_control() {
        struct ExemptScene;
        impl CrawlScene for ExemptScene {
            fn name(&self) -> &str {
                "exempt-scene"
            }
            fn enumerate_controls(&self) -> Vec<InteractiveControl> {
                vec![InteractiveControl {
                    id: "decorative".into(),
                    label: "Decorative".into(),
                    kind: ControlKind::Button,
                    exemption: Some(Exemption::new(
                        "visual-only badge; not a control in product terms",
                    )),
                }]
            }
            fn snapshot(&self) -> SceneSnapshot {
                SceneSnapshot::default()
            }
            fn activate(
                &mut self,
                _control_id: &str,
                _method: ActivationMethod,
            ) -> ActivationOutcome {
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: true,
                }
            }
            fn open_modals(&self) -> Vec<OpenModal> {
                Vec::new()
            }
            fn dismiss_with_escape(&mut self, _modal_id: &str) -> bool {
                false
            }
        }

        let mut scene = ExemptScene;
        let report = crawl_scene(&mut scene);
        assert!(report.passed(), "{:?}", report.failures);
    }

    #[test]
    fn empty_exemption_reason_fails() {
        struct BadExempt;
        impl CrawlScene for BadExempt {
            fn name(&self) -> &str {
                "bad-exempt"
            }
            fn enumerate_controls(&self) -> Vec<InteractiveControl> {
                vec![InteractiveControl {
                    id: "x".into(),
                    label: "X".into(),
                    kind: ControlKind::Button,
                    exemption: Some(Exemption::new("   ")),
                }]
            }
            fn snapshot(&self) -> SceneSnapshot {
                SceneSnapshot::default()
            }
            fn activate(
                &mut self,
                _control_id: &str,
                _method: ActivationMethod,
            ) -> ActivationOutcome {
                ActivationOutcome {
                    after: self.snapshot(),
                    handled: true,
                }
            }
            fn open_modals(&self) -> Vec<OpenModal> {
                Vec::new()
            }
            fn dismiss_with_escape(&mut self, _modal_id: &str) -> bool {
                false
            }
        }

        let mut scene = BadExempt;
        let report = crawl_scene(&mut scene);
        assert!(!report.passed());
    }

    #[test]
    fn checked_in_registry_loads_and_names_the_proving_scene() {
        let registry = CrawlRegistry::load_from_repository()
            .expect("checked-in control-crawl registry must load");
        assert!(
            registry.contains("proving-synthetic"),
            "registry must list proving-synthetic"
        );
        assert!(
            registry.complete_ids().contains(&"proving-synthetic"),
            "proving-synthetic must be complete"
        );
        // Known sealed / modal surfaces stay registered even while pending.
        for required in [
            "omega-front-door",
            "omega-sarah-admission",
            "omega-tester-channel",
            "settings-window",
            "pair-phone",
            "composer-executor-menu",
            "application-menu",
        ] {
            assert!(
                registry.contains(required),
                "registry lost required surface {required}"
            );
        }
    }

    #[test]
    fn copy_lint_flags_multi_sentence_and_respects_allowlist() {
        assert!(is_multi_sentence(
            "This conversation will run on Omega Agent. The executor is free to change."
        ));
        assert!(!is_multi_sentence("Omega Agent"));
        assert!(!is_multi_sentence("v0.2.0"));
        assert!(!is_multi_sentence("Ready"));

        let empty = CopyAllowlist {
            schema: COPY_ALLOWLIST_SCHEMA.into(),
            entries: Vec::new(),
        };
        let essay = "This conversation will run on Omega Agent. The executor is free to change.";
        let offenders = lint_copy([essay, "Ready", "Send"], &empty);
        assert_eq!(offenders, vec![essay.to_string()]);

        let allowed = CopyAllowlist {
            schema: COPY_ALLOWLIST_SCHEMA.into(),
            entries: vec![CopyAllowlistEntry {
                text: essay.into(),
                reason: "temporary until the surface is deleted".into(),
            }],
        };
        assert!(lint_copy([essay], &allowed).is_empty());
    }

    #[test]
    fn status_lint_rejects_prose_and_unregistered_words() {
        assert!(lint_status_words(["Ready", "Blocked", "Offline"]).is_empty());
        assert_eq!(
            lint_status_words(["Awaiting profile", "PRIVATE · PUBLICATION BLOCKED", "Maybe"]),
            vec![
                "Awaiting profile".to_string(),
                "Maybe".to_string(),
                "PRIVATE · PUBLICATION BLOCKED".to_string(),
            ]
        );
    }

    #[test]
    fn presentation_scan_reads_shipped_chrome_and_ignores_test_fixtures() {
        let source = r#"
fn render() {
    div()
        .aria_label("Open Forensics")
        .tooltip(Tooltip::text("Blocked"))
        .child("Send")
        .child(some_expression())
}

#[cfg(test)]
const FIXTURE: &str = "an item-level test gate must not end the scan";

fn render_more() {
    div().child("Retry")
}

#[cfg(test)]
mod tests {
    fn scene() {
        div().aria_label("A fixture label that must not be linted");
    }
}
"#;
        let found = scan_presentation_copy("crates/omega_example/src/example.rs", source);
        assert_eq!(
            found,
            vec![
                PresentationString {
                    file: "crates/omega_example/src/example.rs".into(),
                    slot: PresentationSlot::AccessibleName,
                    text: "Open Forensics".into(),
                },
                PresentationString {
                    file: "crates/omega_example/src/example.rs".into(),
                    slot: PresentationSlot::Tooltip,
                    text: "Blocked".into(),
                },
                PresentationString {
                    file: "crates/omega_example/src/example.rs".into(),
                    slot: PresentationSlot::VisibleLabel,
                    text: "Retry".into(),
                },
                PresentationString {
                    file: "crates/omega_example/src/example.rs".into(),
                    slot: PresentationSlot::VisibleLabel,
                    text: "Send".into(),
                },
            ]
        );
    }

    #[test]
    fn presentation_lint_refuses_exposition_and_overlong_labels() {
        let empty = CopyAllowlist {
            schema: COPY_ALLOWLIST_SCHEMA.into(),
            entries: Vec::new(),
        };
        let essay =
            "This Block is a view over the named source. Visibility does not grant authority.";
        let long_tooltip = "Publication is blocked by the named source authority record";
        let strings = vec![
            PresentationString {
                file: "crates/omega_example/src/example.rs".into(),
                slot: PresentationSlot::VisibleLabel,
                text: essay.into(),
            },
            PresentationString {
                file: "crates/omega_example/src/example.rs".into(),
                slot: PresentationSlot::Tooltip,
                text: long_tooltip.into(),
            },
            PresentationString {
                file: "crates/omega_example/src/example.rs".into(),
                slot: PresentationSlot::VisibleLabel,
                text: "Send".into(),
            },
        ];
        let offenses = lint_presentation_copy(&strings, &empty);
        assert_eq!(offenses.len(), 2, "unexpected offenses: {offenses:?}");
        assert!(offenses.iter().any(|offense| offense.string.text == essay));
        assert!(
            offenses
                .iter()
                .any(|offense| offense.string.text == long_tooltip)
        );

        let allowed = CopyAllowlist {
            schema: COPY_ALLOWLIST_SCHEMA.into(),
            entries: vec![
                CopyAllowlistEntry {
                    text: essay.into(),
                    reason: "example".into(),
                },
                CopyAllowlistEntry {
                    text: long_tooltip.into(),
                    reason: "example".into(),
                },
            ],
        };
        assert!(lint_presentation_copy(&strings, &allowed).is_empty());
    }

    #[test]
    fn omega_owned_sources_are_discovered_by_location() {
        let sources = omega_owned_ui_sources().expect("Omega-owned UI sources");
        for required in [
            "crates/agent_ui/src/omega_status_cue.rs",
            "crates/agent_ui/src/forensics_workbench.rs",
            "crates/agent_ui/src/effective_principal.rs",
            "crates/agent_ui/src/omega_work_detail_surface.rs",
            "crates/agent_ui/src/agent_panel.rs",
            "crates/omega_work_index/src/planning_views.rs",
        ] {
            assert!(
                sources.iter().any(|source| source == required),
                "copy inventory does not cover {required}"
            );
        }
        assert!(
            !sources
                .iter()
                .any(|source| source == "crates/omega_control_crawl/src/omega_control_crawl.rs"),
            "the lint quotes its own markers and is not a chrome surface"
        );
    }

    /// The inventory in `docs/omega/oaw-014-concise-copy-inventory.md` is only
    /// a disposition until something refuses new exposition. This is that
    /// refusal: it runs over the real tree, so a new Omega destination is
    /// covered the moment its file exists.
    #[test]
    fn omega_owned_presentation_copy_is_concise() {
        let allowlist =
            CopyAllowlist::load_from_repository().expect("checked-in copy allowlist must load");
        let offenses = lint_repository_presentation_copy(&allowlist).expect("repository copy scan");
        assert!(
            offenses.is_empty(),
            "Omega-owned presentation copy is not concise: {offenses:#?}"
        );
    }

    #[test]
    fn checked_in_copy_allowlist_loads() {
        let allowlist =
            CopyAllowlist::load_from_repository().expect("checked-in copy allowlist must load");
        assert_eq!(allowlist.schema, COPY_ALLOWLIST_SCHEMA);
    }

    #[test]
    fn registry_rejects_wrong_schema_and_duplicate_ids() {
        let bad_schema =
            r#"{"schema":"nope","surfaces":[{"id":"a","kind":"x","status":"complete"}]}"#;
        assert!(CrawlRegistry::parse(bad_schema).is_err());

        let dup = r#"{
            "schema":"openagents.omega.control-crawl-registry.v1",
            "surfaces":[
                {"id":"a","kind":"x","status":"complete"},
                {"id":"a","kind":"x","status":"complete"}
            ]
        }"#;
        assert!(CrawlRegistry::parse(dup).is_err());
    }
}
