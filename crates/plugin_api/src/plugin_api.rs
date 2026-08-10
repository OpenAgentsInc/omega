//! The registration contract between the Omega application and its plugins.
//!
//! A plugin is a statically linked crate family that implements [`OmegaPlugin`]
//! and contributes everything else — background services, agent tools, settings
//! pages, panels, card schemas, and network host declarations — through one
//! [`PluginRegistry`] populated at startup. Core crates read the registry; they
//! never name a plugin. The only file that names plugins is
//! `crates/omega/src/plugins.rs`.
//!
//! Plugins do not implement venue connectivity. Nautilus owns market-data and
//! execution adapters behind a continuous versioned event stream. Its fast
//! typed, versioned command channel carries execution operations. This registry
//! remains the extension seam for governance tools, cards, settings, panels,
//! review drivers, and capability observations.

use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use gpui::{AnyElement, AnyView, App, Global, Window};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use agent_wakeup::WakeupSource;
pub use carry_surface::{
    CARRY_SURFACE_SCHEMA, CarryCostBreakdown, CarrySurface, CarrySurfaceError, CarrySurfaceInput,
    CarrySurfaceProvider, CarrySurfaceRequest, ContractKind, ExpectedFundingPayment,
    ExpectedSlippage, FeeSchedule, MeasurementWindow, PositionSide, SettlementCadence,
    normalize_carry,
};
pub use review_accounting::{ReviewTokenUsage, ReviewToolCall};

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

/// A platform-known account mode. A venue-specific value that is not in this
/// enum remains raw and unverified until the platform learns it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueAccountMode {
    SingleAccount,
    UnifiedAccount,
    PortfolioMargin,
}

/// A platform-known margin mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueMarginMode {
    VenueManaged,
    Cross,
    Isolated,
    Portfolio,
}

/// A raw venue observation paired with its typed interpretation when known.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedVenueMode<Mode> {
    pub typed: Option<Mode>,
    pub raw: String,
}

impl<Mode> ObservedVenueMode<Mode> {
    pub fn known(typed: Mode, raw: impl Into<String>) -> Self {
        Self {
            typed: Some(typed),
            raw: raw.into(),
        }
    }

    pub fn unknown(raw: impl Into<String>) -> Self {
        Self {
            typed: None,
            raw: raw.into(),
        }
    }
}

/// An effectful venue action whose availability must be probed before use.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueActionClass {
    AssetSwap,
    StrategyExecution,
    OrderPlacement,
    OrderCancellation,
    Transfer,
    Withdrawal,
    AgentApproval,
    BuilderFeeApproval,
}

impl VenueActionClass {
    pub const fn label(self) -> &'static str {
        match self {
            Self::AssetSwap => "asset_swap",
            Self::StrategyExecution => "strategy_execution",
            Self::OrderPlacement => "order_placement",
            Self::OrderCancellation => "order_cancellation",
            Self::Transfer => "transfer",
            Self::Withdrawal => "withdrawal",
            Self::AgentApproval => "agent_approval",
            Self::BuilderFeeApproval => "builder_fee_approval",
        }
    }
}

impl fmt::Display for VenueActionClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum VenueActionStatus {
    Supported,
    Disabled { reason: String },
    Unknown { raw: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProbedVenueAssumption<Value> {
    pub value: Value,
    pub probed_at_ms: i64,
}

impl<Value> ProbedVenueAssumption<Value> {
    pub fn new(value: Value, probed_at_ms: i64) -> Self {
        Self {
            value,
            probed_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VenueActionCapability {
    pub action_class: VenueActionClass,
    pub status: VenueActionStatus,
}

/// The complete observed effectful surface for one venue.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VenueCapabilities {
    pub venue_id: String,
    pub account_mode: ProbedVenueAssumption<ObservedVenueMode<VenueAccountMode>>,
    pub margin_mode: ProbedVenueAssumption<ObservedVenueMode<VenueMarginMode>>,
    pub actions: Vec<ProbedVenueAssumption<VenueActionCapability>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCapabilityVerificationStatus {
    Verified,
    Unverified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VenueCapabilityVerification {
    pub status: VenueCapabilityVerificationStatus,
    pub stale: bool,
    pub oldest_probed_at_ms: Option<i64>,
    pub newest_probed_at_ms: Option<i64>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VenueCapabilityReport {
    pub capabilities: Option<VenueCapabilities>,
    pub verification: VenueCapabilityVerification,
}

#[derive(Clone, Debug, Eq, Error, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VenueCapabilityError {
    #[error("{venue_id} capabilities have not been probed")]
    CapabilitiesNotProbed { venue_id: String },
    #[error("{venue_id} account mode is unknown: {raw}")]
    UnknownAccountMode {
        venue_id: String,
        raw: String,
        probed_at_ms: i64,
    },
    #[error("{venue_id} margin mode is unknown: {raw}")]
    UnknownMarginMode {
        venue_id: String,
        raw: String,
        probed_at_ms: i64,
    },
    #[error("{venue_id} did not probe action class {action_class}")]
    ActionNotProbed {
        venue_id: String,
        action_class: VenueActionClass,
    },
    #[error("{venue_id} action class {action_class} is unknown: {raw}")]
    UnknownAction {
        venue_id: String,
        action_class: VenueActionClass,
        raw: String,
        probed_at_ms: i64,
    },
    #[error("{venue_id} action class {action_class} is disabled: {reason}")]
    ActionDisabled {
        venue_id: String,
        action_class: VenueActionClass,
        reason: String,
        probed_at_ms: i64,
    },
    #[error(
        "{venue_id} action class {action_class} relies on a stale capability probe from {oldest_probed_at_ms}"
    )]
    StaleProbe {
        venue_id: String,
        action_class: VenueActionClass,
        oldest_probed_at_ms: i64,
        checked_at_ms: i64,
        maximum_age_ms: i64,
    },
    #[error("capability freshness window must not be negative")]
    InvalidFreshnessWindow,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum VenueCapabilityPublicationError {
    #[error("venue ID must not be empty")]
    EmptyVenueId,
    #[error("venue capability probe timestamps must not be negative")]
    NegativeProbeTimestamp,
    #[error("venue capability action class {action_class} was reported more than once")]
    DuplicateActionClass { action_class: VenueActionClass },
}

#[derive(Clone, Default)]
pub struct VenueCapabilityStore {
    capabilities: Arc<RwLock<HashMap<String, VenueCapabilities>>>,
}

impl VenueCapabilityStore {
    pub fn publish(
        &self,
        capabilities: VenueCapabilities,
    ) -> Result<(), VenueCapabilityPublicationError> {
        if capabilities.venue_id.trim().is_empty() {
            return Err(VenueCapabilityPublicationError::EmptyVenueId);
        }
        if capabilities.account_mode.probed_at_ms < 0
            || capabilities.margin_mode.probed_at_ms < 0
            || capabilities
                .actions
                .iter()
                .any(|action| action.probed_at_ms < 0)
        {
            return Err(VenueCapabilityPublicationError::NegativeProbeTimestamp);
        }
        let mut action_classes = HashSet::new();
        for action in &capabilities.actions {
            if !action_classes.insert(action.value.action_class) {
                return Err(VenueCapabilityPublicationError::DuplicateActionClass {
                    action_class: action.value.action_class,
                });
            }
        }
        self.capabilities
            .write()
            .insert(capabilities.venue_id.clone(), capabilities);
        Ok(())
    }

    pub fn snapshot(&self, venue_id: &str) -> Option<VenueCapabilities> {
        self.capabilities.read().get(venue_id).cloned()
    }

    pub fn guard(
        &self,
        venue_id: impl Into<String>,
        action_class: VenueActionClass,
        maximum_age_ms: i64,
    ) -> VenueCapabilityGuard {
        VenueCapabilityGuard {
            store: self.clone(),
            venue_id: venue_id.into(),
            action_class,
            maximum_age_ms,
        }
    }

    pub fn report(
        &self,
        venue_id: &str,
        checked_at_ms: i64,
        maximum_age_ms: i64,
    ) -> VenueCapabilityReport {
        let Some(capabilities) = self.snapshot(venue_id) else {
            return VenueCapabilityReport {
                capabilities: None,
                verification: VenueCapabilityVerification {
                    status: VenueCapabilityVerificationStatus::Unverified,
                    stale: false,
                    oldest_probed_at_ms: None,
                    newest_probed_at_ms: None,
                    reasons: vec![format!("{venue_id} capabilities have not been probed")],
                },
            };
        };
        let mut reasons = Vec::new();
        if capabilities.account_mode.value.typed.is_none() {
            reasons.push(format!(
                "unknown account mode: {}",
                capabilities.account_mode.value.raw
            ));
        }
        if capabilities.margin_mode.value.typed.is_none() {
            reasons.push(format!(
                "unknown margin mode: {}",
                capabilities.margin_mode.value.raw
            ));
        }
        for action in &capabilities.actions {
            if let VenueActionStatus::Unknown { raw } = &action.value.status {
                reasons.push(format!(
                    "unknown {} capability: {raw}",
                    action.value.action_class
                ));
            }
        }
        let probe_timestamps = std::iter::once(capabilities.account_mode.probed_at_ms)
            .chain(std::iter::once(capabilities.margin_mode.probed_at_ms))
            .chain(
                capabilities
                    .actions
                    .iter()
                    .map(|action| action.probed_at_ms),
            )
            .collect::<Vec<_>>();
        let oldest_probed_at_ms = probe_timestamps.iter().copied().min();
        let newest_probed_at_ms = probe_timestamps.iter().copied().max();
        let stale = if maximum_age_ms < 0 {
            reasons.push("capability freshness window is invalid".to_string());
            true
        } else {
            probe_timestamps.iter().any(|probed_at_ms| {
                checked_at_ms < *probed_at_ms
                    || checked_at_ms.saturating_sub(*probed_at_ms) > maximum_age_ms
            })
        };
        if stale {
            reasons.push("one or more capability probes are stale".to_string());
        }
        VenueCapabilityReport {
            capabilities: Some(capabilities),
            verification: VenueCapabilityVerification {
                status: if reasons.is_empty() {
                    VenueCapabilityVerificationStatus::Verified
                } else {
                    VenueCapabilityVerificationStatus::Unverified
                },
                stale,
                oldest_probed_at_ms,
                newest_probed_at_ms,
                reasons,
            },
        }
    }
}

#[derive(Clone)]
pub struct VenueCapabilityGuard {
    store: VenueCapabilityStore,
    venue_id: String,
    action_class: VenueActionClass,
    maximum_age_ms: i64,
}

impl VenueCapabilityGuard {
    pub fn require_effectful(&self, checked_at_ms: i64) -> Result<(), VenueCapabilityError> {
        if self.maximum_age_ms < 0 {
            return Err(VenueCapabilityError::InvalidFreshnessWindow);
        }
        let capabilities = self.store.snapshot(&self.venue_id).ok_or_else(|| {
            VenueCapabilityError::CapabilitiesNotProbed {
                venue_id: self.venue_id.clone(),
            }
        })?;
        if capabilities.account_mode.value.typed.is_none() {
            return Err(VenueCapabilityError::UnknownAccountMode {
                venue_id: self.venue_id.clone(),
                raw: capabilities.account_mode.value.raw,
                probed_at_ms: capabilities.account_mode.probed_at_ms,
            });
        }
        if capabilities.margin_mode.value.typed.is_none() {
            return Err(VenueCapabilityError::UnknownMarginMode {
                venue_id: self.venue_id.clone(),
                raw: capabilities.margin_mode.value.raw,
                probed_at_ms: capabilities.margin_mode.probed_at_ms,
            });
        }
        let action = capabilities
            .actions
            .iter()
            .find(|action| action.value.action_class == self.action_class)
            .ok_or_else(|| VenueCapabilityError::ActionNotProbed {
                venue_id: self.venue_id.clone(),
                action_class: self.action_class,
            })?;
        match &action.value.status {
            VenueActionStatus::Supported => {}
            VenueActionStatus::Disabled { reason } => {
                return Err(VenueCapabilityError::ActionDisabled {
                    venue_id: self.venue_id.clone(),
                    action_class: self.action_class,
                    reason: reason.clone(),
                    probed_at_ms: action.probed_at_ms,
                });
            }
            VenueActionStatus::Unknown { raw } => {
                return Err(VenueCapabilityError::UnknownAction {
                    venue_id: self.venue_id.clone(),
                    action_class: self.action_class,
                    raw: raw.clone(),
                    probed_at_ms: action.probed_at_ms,
                });
            }
        }
        let oldest_probed_at_ms = capabilities
            .account_mode
            .probed_at_ms
            .min(capabilities.margin_mode.probed_at_ms)
            .min(action.probed_at_ms);
        if checked_at_ms < oldest_probed_at_ms
            || checked_at_ms.saturating_sub(oldest_probed_at_ms) > self.maximum_age_ms
        {
            return Err(VenueCapabilityError::StaleProbe {
                venue_id: self.venue_id.clone(),
                action_class: self.action_class,
                oldest_probed_at_ms,
                checked_at_ms,
                maximum_age_ms: self.maximum_age_ms,
            });
        }
        Ok(())
    }

    pub fn report(&self, checked_at_ms: i64) -> VenueCapabilityReport {
        self.store
            .report(&self.venue_id, checked_at_ms, self.maximum_age_ms)
    }
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
    pub completed_at_ms: i64,
    pub wall_clock_ms: u64,
    pub model_id: String,
    pub token_usage: ReviewTokenUsage,
    pub tool_calls: Vec<ReviewToolCall>,
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

/// A native renderer for one exact, versioned tool-result schema.
pub struct CardRendererRegistration {
    pub plugin_id: &'static str,
    pub schema: &'static str,
    pub render: Rc<dyn Fn(&serde_json::Value, &App) -> Option<AnyElement>>,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CardRendererRegistrationError {
    #[error("card renderer schema `{schema}` is already registered")]
    DuplicateSchema { schema: &'static str },
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
    card_renderers: Vec<Rc<CardRendererRegistration>>,
    review_drivers: Vec<Rc<dyn SessionReviewDriver>>,
    venue_capabilities: VenueCapabilityStore,
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
            card_renderers: Vec::new(),
            review_drivers: Vec::new(),
            venue_capabilities: VenueCapabilityStore::default(),
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

    pub fn add_card_renderer(
        &mut self,
        renderer: CardRendererRegistration,
    ) -> Result<(), CardRendererRegistrationError> {
        if self
            .card_renderers
            .iter()
            .any(|registered| registered.schema == renderer.schema)
        {
            return Err(CardRendererRegistrationError::DuplicateSchema {
                schema: renderer.schema,
            });
        }
        self.card_renderers.push(Rc::new(renderer));
        Ok(())
    }

    pub fn card_renderer(&self, schema: &str) -> Option<Rc<CardRendererRegistration>> {
        self.card_renderers
            .iter()
            .find(|renderer| renderer.schema == schema)
            .cloned()
    }

    pub fn add_review_driver(&mut self, driver: Rc<dyn SessionReviewDriver>) {
        self.review_drivers.push(driver);
    }

    pub fn review_drivers(&self) -> &[Rc<dyn SessionReviewDriver>] {
        &self.review_drivers
    }

    pub fn venue_capabilities(&self) -> VenueCapabilityStore {
        self.venue_capabilities.clone()
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

    #[test]
    fn card_renderer_schemas_are_exact_and_unique() {
        let mut registry = PluginRegistry::new(PathBuf::from("/data"));
        registry
            .add_card_renderer(CardRendererRegistration {
                plugin_id: "example",
                schema: "omega.example.account.v1",
                render: Rc::new(|_, _| None),
            })
            .expect("register renderer");
        assert!(registry.card_renderer("omega.example.account.v1").is_some());
        assert!(registry.card_renderer("omega.example.account.v2").is_none());
        assert_eq!(
            registry
                .add_card_renderer(CardRendererRegistration {
                    plugin_id: "other",
                    schema: "omega.example.account.v1",
                    render: Rc::new(|_, _| None),
                })
                .expect_err("duplicate schema must be refused"),
            CardRendererRegistrationError::DuplicateSchema {
                schema: "omega.example.account.v1",
            }
        );
    }

    fn capabilities_at(
        probed_at_ms: i64,
        account_mode: ObservedVenueMode<VenueAccountMode>,
    ) -> VenueCapabilities {
        VenueCapabilities {
            venue_id: "example".to_string(),
            account_mode: ProbedVenueAssumption::new(account_mode, probed_at_ms),
            margin_mode: ProbedVenueAssumption::new(
                ObservedVenueMode::known(VenueMarginMode::Cross, "cross"),
                probed_at_ms,
            ),
            actions: vec![ProbedVenueAssumption::new(
                VenueActionCapability {
                    action_class: VenueActionClass::OrderPlacement,
                    status: VenueActionStatus::Supported,
                },
                probed_at_ms,
            )],
        }
    }

    #[test]
    fn effectful_guard_refuses_unknown_account_mode() {
        let store = VenueCapabilityStore::default();
        store
            .publish(capabilities_at(
                1_000,
                ObservedVenueMode::unknown("venue_mode_47"),
            ))
            .expect("publish capabilities");
        let error = store
            .guard("example", VenueActionClass::OrderPlacement, 1_000)
            .require_effectful(1_500)
            .expect_err("unknown mode must fail closed");
        assert_eq!(
            error,
            VenueCapabilityError::UnknownAccountMode {
                venue_id: "example".to_string(),
                raw: "venue_mode_47".to_string(),
                probed_at_ms: 1_000,
            }
        );
    }

    #[test]
    fn stale_probe_is_labeled_and_refused() {
        let store = VenueCapabilityStore::default();
        store
            .publish(capabilities_at(
                1_000,
                ObservedVenueMode::known(VenueAccountMode::UnifiedAccount, "unified"),
            ))
            .expect("publish capabilities");
        let guard = store.guard("example", VenueActionClass::OrderPlacement, 500);
        let report = guard.report(1_501);
        assert_eq!(
            report.verification.status,
            VenueCapabilityVerificationStatus::Unverified
        );
        assert!(report.verification.stale);
        assert!(matches!(
            guard.require_effectful(1_501),
            Err(VenueCapabilityError::StaleProbe {
                oldest_probed_at_ms: 1_000,
                ..
            })
        ));
    }
}
