//! The registration contract between the Omega application and its plugins.
//!
//! A plugin is a statically linked crate family that implements [`OmegaPlugin`]
//! and contributes everything else — background services, agent tools, settings
//! pages, panels, card schemas, and network host declarations — through one
//! [`PluginRegistry`] populated at startup. Core crates read the registry; they
//! never name a plugin. The only file that names plugins is
//! `crates/omega/src/plugins.rs`.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::{AnyView, App, Global, Window};

pub use agent_wakeup::WakeupSource;

/// A network protocol a plugin host declaration covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Https,
    Wss,
}

/// The highest network tier a plugin's automated behavior has been proven on.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Maturity {
    Regtest,
    Signet,
    Testnet,
    Mainnet,
}

/// One network host a plugin is allowed to reach, declared with its purpose so
/// the endpoint evidence can show why the host exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostDeclaration {
    pub host: &'static str,
    pub purpose: &'static str,
    pub protocols: &'static [Protocol],
}

/// A plugin's identity and its complete network surface. The union of
/// registered manifests' host declarations is the source of allowed plugin
/// hosts; enforcement stays in platform code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub maturity: Maturity,
    pub hosts: &'static [HostDeclaration],
}

/// The registration trait every plugin implements exactly once.
pub trait OmegaPlugin: 'static {
    fn manifest(&self) -> &'static PluginManifest;
    fn register(&self, registry: &mut PluginRegistry, cx: &mut App);
}

/// How a plugin-claimed review session schedules its unattended turns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewCadence {
    /// Turns run only when the plugin emits a pending wakeup event.
    EventDriven,
    /// Turns run on a fixed interval in addition to pending events.
    Interval { seconds: u64 },
}

/// The outcome of one unattended review turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewTurnOutcome {
    Completed,
    Failed,
}

/// Bounded, venue-neutral evidence measured from one completed review turn.
#[derive(Clone, Debug)]
pub struct ReviewTurnEvidence {
    pub at_ms: i64,
    pub source: WakeupSource,
    pub reasoning_note_present: bool,
    /// How many calls to the driver's [`SessionReviewDriver::evidence_tool_names`]
    /// the turn made.
    pub tracked_tool_calls: u32,
    pub tokens_used: u64,
}

/// A plugin-owned driver for unattended review turns on a claimed session.
///
/// The agent's wakeup scheduler consults registered drivers instead of naming
/// any plugin: the driver that claims a session supplies its cadence, token
/// budget, pending events, and turn instructions, and receives the turn
/// outcomes and measured evidence back.
pub trait SessionReviewDriver: 'static {
    /// The cadence for this session, or `None` when the driver has not claimed
    /// it. An error is reported but treated as an unclaimed session.
    fn review_cadence(&self, session_id: &str, cx: &App) -> Result<Option<ReviewCadence>, String>;

    /// The per-turn token ceiling for review turns this driver owns.
    fn review_token_budget(&self) -> u64;

    /// A pending event wakeup for this session, with its instruction text.
    fn pending_wakeup(&self, session_id: &str, cx: &App) -> Option<(WakeupSource, String)>;

    /// The full instruction for a review turn, or `Ok(None)` when the session
    /// is no longer claimed.
    fn review_instruction(
        &self,
        session_id: &str,
        now_ms: i64,
        trigger: &str,
        cx: &App,
    ) -> Result<Option<String>, String>;

    /// Acknowledge a completed event wakeup so it is not redelivered.
    fn acknowledge_wakeup(
        &self,
        session_id: &str,
        source: &WakeupSource,
        instruction: &str,
        cx: &App,
    ) -> bool;

    /// Record one review turn outcome for operator history.
    fn record_review_turn(
        &self,
        session_id: &str,
        at_ms: i64,
        source: WakeupSource,
        outcome: ReviewTurnOutcome,
        cx: &App,
    ) -> bool;

    /// Tool names whose calls the agent counts into [`ReviewTurnEvidence`].
    fn evidence_tool_names(&self) -> &'static [&'static str];

    /// Record the measured evidence for one completed review turn.
    fn record_review_evidence(
        &self,
        session_id: &str,
        evidence: ReviewTurnEvidence,
        cx: &App,
    ) -> bool;
}

/// A settings sub-page contributed by a plugin. The settings surface builds
/// the page view once per settings window through `build` and places the link
/// under `section`.
pub struct SettingsPageRegistration {
    pub plugin_id: &'static str,
    /// The section header the page link appears under.
    pub section: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub search_aliases: &'static [&'static str],
    /// A stable key identifying the page within the settings surface.
    pub page_key: &'static str,
    pub build: Rc<dyn Fn(&mut Window, &mut App) -> AnyView>,
}

/// An app-lifetime background service a plugin started during registration,
/// recorded for operator visibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundServiceRegistration {
    pub plugin_id: &'static str,
    pub service_id: &'static str,
    pub description: &'static str,
}

/// A versioned card schema a plugin's tools emit. Cards render from schema
/// data on the platform side; this records which schemas a plugin owns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CardSchemaRegistration {
    pub plugin_id: &'static str,
    pub schema: &'static str,
}

/// Everything a plugin registers at startup, and everything core crates read
/// back. Registration surfaces whose types belong to a consuming platform
/// crate (agent tools, workbench panels) go through the typed extension slots
/// via [`PluginRegistry::add_extension`].
pub struct PluginRegistry {
    data_root: PathBuf,
    manifests: Vec<&'static PluginManifest>,
    settings_pages: Vec<Rc<SettingsPageRegistration>>,
    background_services: Vec<BackgroundServiceRegistration>,
    card_schemas: Vec<CardSchemaRegistration>,
    review_drivers: Vec<Rc<dyn SessionReviewDriver>>,
    extensions: HashMap<TypeId, Vec<Rc<dyn Any>>>,
}

impl PluginRegistry {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            manifests: Vec::new(),
            settings_pages: Vec::new(),
            background_services: Vec::new(),
            card_schemas: Vec::new(),
            review_drivers: Vec::new(),
            extensions: HashMap::new(),
        }
    }

    /// Register one plugin: its manifest is recorded first, then the plugin
    /// contributes its surfaces.
    pub fn register_plugin(&mut self, plugin: &dyn OmegaPlugin, cx: &mut App) {
        self.manifests.push(plugin.manifest());
        plugin.register(self, cx);
    }

    pub fn manifests(&self) -> &[&'static PluginManifest] {
        &self.manifests
    }

    /// The union of every registered manifest's host declarations. This is
    /// the single source of allowed plugin hosts.
    pub fn allowed_hosts(&self) -> impl Iterator<Item = &'static HostDeclaration> + '_ {
        self.manifests
            .iter()
            .flat_map(|manifest| manifest.hosts.iter())
    }

    pub fn add_settings_page(&mut self, page: SettingsPageRegistration) {
        self.settings_pages.push(Rc::new(page));
    }

    pub fn settings_pages(&self) -> &[Rc<SettingsPageRegistration>] {
        &self.settings_pages
    }

    pub fn add_background_service(&mut self, service: BackgroundServiceRegistration) {
        self.background_services.push(service);
    }

    pub fn background_services(&self) -> &[BackgroundServiceRegistration] {
        &self.background_services
    }

    pub fn add_card_schema(&mut self, card_schema: CardSchemaRegistration) {
        self.card_schemas.push(card_schema);
    }

    pub fn card_schemas(&self) -> &[CardSchemaRegistration] {
        &self.card_schemas
    }

    pub fn add_review_driver(&mut self, driver: Rc<dyn SessionReviewDriver>) {
        self.review_drivers.push(driver);
    }

    pub fn review_drivers(&self) -> &[Rc<dyn SessionReviewDriver>] {
        &self.review_drivers
    }

    /// Register a consumer-typed surface. The concrete type is owned by the
    /// platform crate that consumes it (for example the agent's tool
    /// registration type), which keeps this crate free of those dependencies
    /// while registration stays typed at both ends.
    pub fn add_extension<T: Any>(&mut self, extension: T) {
        self.extensions
            .entry(TypeId::of::<T>())
            .or_default()
            .push(Rc::new(extension));
    }

    /// Read back every registered extension of one concrete type.
    pub fn extensions<T: Any>(&self) -> Vec<Rc<T>> {
        self.extensions
            .get(&TypeId::of::<T>())
            .into_iter()
            .flatten()
            .filter_map(|extension| extension.clone().downcast::<T>().ok())
            .collect()
    }

    /// The per-plugin data directory, `plugins/<id>/` under the app data dir,
    /// so plugin stores never collide and a reset is a directory delete.
    pub fn plugin_data_directory(&self, plugin_id: &str) -> PathBuf {
        self.data_root.join("plugins").join(plugin_id)
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }
}

struct GlobalPluginRegistry(Rc<PluginRegistry>);

impl Global for GlobalPluginRegistry {}

/// Store the fully populated registry for the app's lifetime.
pub fn init_global(registry: PluginRegistry, cx: &mut App) {
    cx.set_global(GlobalPluginRegistry(Rc::new(registry)));
}

/// The registry populated at startup, or `None` before plugin registration
/// (and in tests that did not set one up).
pub fn registry(cx: &App) -> Option<Rc<PluginRegistry>> {
    cx.try_global::<GlobalPluginRegistry>()
        .map(|global| global.0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_data_directory_is_namespaced_by_plugin_id() {
        let registry = PluginRegistry::new(PathBuf::from("/data"));
        assert_eq!(
            registry.plugin_data_directory("example"),
            PathBuf::from("/data/plugins/example")
        );
    }

    #[test]
    fn allowed_hosts_are_the_union_of_registered_manifests() {
        static FIRST: PluginManifest = PluginManifest {
            id: "first",
            name: "First",
            version: "0.0.0",
            maturity: Maturity::Signet,
            hosts: &[HostDeclaration {
                host: "api.first.example",
                purpose: "first REST",
                protocols: &[Protocol::Https],
            }],
        };
        static SECOND: PluginManifest = PluginManifest {
            id: "second",
            name: "Second",
            version: "0.0.0",
            maturity: Maturity::Testnet,
            hosts: &[HostDeclaration {
                host: "stream.second.example",
                purpose: "second stream",
                protocols: &[Protocol::Wss],
            }],
        };
        let mut registry = PluginRegistry::new(PathBuf::from("/data"));
        registry.manifests.push(&FIRST);
        registry.manifests.push(&SECOND);
        let hosts: Vec<&str> = registry.allowed_hosts().map(|host| host.host).collect();
        assert_eq!(hosts, ["api.first.example", "stream.second.example"]);
    }

    #[test]
    fn extensions_round_trip_by_concrete_type() {
        struct ToolSurface(&'static str);
        struct PanelSurface(&'static str);
        let mut registry = PluginRegistry::new(PathBuf::from("/data"));
        registry.add_extension(ToolSurface("tools"));
        registry.add_extension(PanelSurface("panel"));
        registry.add_extension(ToolSurface("more tools"));
        let tools = registry.extensions::<ToolSurface>();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].0, "tools");
        assert_eq!(tools[1].0, "more tools");
        let panels = registry.extensions::<PanelSurface>();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].0, "panel");
        assert!(registry.extensions::<u32>().is_empty());
    }
}
