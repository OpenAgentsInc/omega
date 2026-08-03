use std::cell::RefCell;
use std::collections::{HashMap as StdHashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use acp_thread::{AgentThreadEntry, ThreadStatus};
use chrono::Utc;
use gpui::{AnyWindowHandle, App, AppContext, AsyncApp, Entity, WeakEntity};
use omega_effectd::{
    ConversationIdentity, HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    OmegaEffectdHostHandler, OpenAgentsSession, SARAH_METHOD_BOOTSTRAP, SARAH_METHOD_DEVICE_GRANTS,
    SARAH_METHOD_INTERRUPT_TURN, SARAH_METHOD_RENEW_DEVICE_GRANT, SARAH_METHOD_REVOKE_DEVICE_GRANT,
    SARAH_METHOD_ROOM_SNAPSHOT, SARAH_METHOD_SEND_MESSAGE, SARAH_METHOD_SESSION_STATUS,
    SarahConversationClient, SarahConversationConfig, VerifiedOpenAgentsSession,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use workspace::{AppState, Workspace};

use crate::agent_panel::CreateThreadOptions;
use crate::{
    Agent, AgentPanel, AgentThreadSource, ConversationView, ThreadId,
    agent_connection_store::AgentConnectionStatus, thread_metadata_store::ThreadMetadataStore,
};

const SUPERVISED_WORKSPACE_REF: &str = "workspace.omega.supervised";
const CODEX_LOCAL_LANE: &str = "codex-local";
const CLAUDE_LOCAL_LANE: &str = "claude-local";
const CODEX_AGENT_ID: &str = "codex-acp";
const CLAUDE_AGENT_ID: &str = "claude-acp";
const THREAD_CONNECT_ATTEMPTS: usize = 100;
const THREAD_CONNECT_INTERVAL: Duration = Duration::from_millis(100);
const CLAUDE_TURN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const ACP_CANCEL_GRACE_PERIOD: Duration = Duration::from_secs(10);
const MAX_ASSISTANT_TEXT_BYTES: usize = 24 * 1024;
const MAX_EVIDENCE_TURNS: usize = 48;
const MAX_TOTAL_ASSISTANT_TEXT_BYTES: usize = 6 * 1024;
const MAX_DEVICE_TRANSCRIPT_MESSAGES: usize = 64;
const MAX_DEVICE_TRANSCRIPT_TEXT_BYTES: usize = 8 * 1024;
const MAX_DEVICE_TRANSCRIPT_TOTAL_BYTES: usize = 32 * 1024;
const MAX_DEVICE_THREADS: usize = 64;
const MAX_DEVICE_RUNS: usize = 64;
const MAX_DEVICE_LABEL_BYTES: usize = 256;
const DEVICE_SNAPSHOT_FRAME_RESERVE_BYTES: usize = 1_024;
const DEVICE_TRANSCRIPT_PUBLISH_INTERVAL: Duration = Duration::from_millis(250);
const ISSUE31_AGENT_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// omega#124. How often the panel's own threads are re-projected for the mirror.
///
/// A second was long enough that a person typing on the desktop watched the
/// phone lag behind them. The work is a diff against what was already
/// published, so an idle desktop publishes nothing and the cost of asking more
/// often is a comparison.
const PANEL_PROJECTION_INTERVAL: Duration = Duration::from_millis(150);
const CORRELATION_SCHEMA: &str = "openagents.omega.full_auto_host_correlation.v1";
const CORRELATION_FILE: &str = "full-auto-host-correlation.json";

/// OMEGA-DELTA-0021. Which `omega-effectd` lane run each host-bridge thread
/// belongs to, so the thread surface can disclose it.
///
/// A read-mostly index, not a store. It is derived twice from the correlation
/// journal — once when the journal is loaded at startup, and again on every
/// write — so it cannot outlive or disagree with the durable record. omega#77's
/// falsifier names a *new durable* store for disclosure; this is neither new
/// nor durable, and deleting the file it mirrors empties it.
///
/// It exists because `HostBridgeState` lives inside the host handler's closure
/// and is not reachable from a render, while the disclosure has to be readable
/// wherever a thread is drawn.
static ENGINE_LANE_RUNS: LazyLock<Mutex<StdHashMap<ThreadId, String>>> =
    LazyLock::new(|| Mutex::new(StdHashMap::new()));

/// The lane index a set of host threads implies.
fn engine_lane_runs_from(threads: &[HostThread]) -> StdHashMap<ThreadId, String> {
    threads
        .iter()
        .map(|thread| (thread.thread_id, thread.operation_ref.clone()))
        .collect()
}

/// Replace the lane index with what the correlation journal now says.
///
/// Wholesale replacement rather than insertion: a thread dropped from the
/// journal must stop being disclosed as a lane run, and an incremental index
/// would keep disclosing it.
fn republish_engine_lane_runs(threads: &[HostThread]) {
    let runs = engine_lane_runs_from(threads);
    match ENGINE_LANE_RUNS.lock() {
        Ok(mut index) => *index = runs,
        Err(poisoned) => *poisoned.into_inner() = runs,
    }
}

/// Publish one lane correlation, for a harness that has no engine to run.
///
/// `OMEGA-DELTA-0021`'s engine-lane disclosure is normally reached only by a
/// real `omega-effectd` run, which a rendering harness cannot start. This is
/// the seam that lets the *rendered* engine-lane line be captured without
/// weakening the production path: it is behind `test-support`, so no shipped
/// build can reach it, and it writes the same index the correlation journal
/// writes.
#[cfg(any(test, feature = "test-support"))]
pub fn publish_engine_lane_run_for_tests(thread_id: ThreadId, operation_ref: String) {
    match ENGINE_LANE_RUNS.lock() {
        Ok(mut index) => {
            index.insert(thread_id, operation_ref);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(thread_id, operation_ref);
        }
    }
}

/// Persist one lane correlation through the production writer, for a harness
/// that has no engine to run.
///
/// [`publish_engine_lane_run_for_tests`] writes the process-local index, which
/// a restart empties. This writes the durable half — the same
/// `CorrelationJournal`, in the same schema, at the same path
/// [`omega_effectd_host_handler`] reads at startup — so a *second process* can
/// be shown rebuilding the disclosure from disk rather than from a static that
/// happened to survive. Behind `test-support`, so no shipped build can reach it.
#[cfg(any(test, feature = "test-support"))]
pub fn persist_engine_lane_run_for_tests(
    thread_id: ThreadId,
    operation_ref: String,
) -> anyhow::Result<()> {
    let state = HostBridgeState {
        workspace: None,
        threads: vec![HostThread {
            workspace_ref: SUPERVISED_WORKSPACE_REF.to_string(),
            lane: CODEX_LOCAL_LANE.to_string(),
            operation_ref,
            thread_id,
            conversation: None,
            turns: Vec::new(),
            revision: 1,
        }],
        correlation_path: correlation_journal_path(),
        load_error: None,
        sarah_conversation: None,
        device_bridge: None,
        device_projection: None,
    };
    persist_correlation_journal(&state)
}

/// Where the correlation journal lives.
fn correlation_journal_path() -> PathBuf {
    paths::data_dir().join("openagents").join(CORRELATION_FILE)
}

/// Read the correlation journal and refill the lane index from it.
///
/// OMEGA-DELTA-0021's restart edge, in one place. A freshly started process has
/// an empty lane index, and this is what refills it, so a thread resumed after
/// a restart still discloses the lane that owns it.
///
/// [`omega_effectd_host_handler`] calls this at startup and keeps the threads
/// for its own state; [`reload_engine_lane_runs_from_disk`] calls it for a
/// caller that only needs the index. Neither is a copy of the other, so what a
/// cold process is observed doing here is what the shipped startup does.
fn load_journal_and_republish(path: &Path) -> (Vec<HostThread>, Option<String>) {
    let (threads, load_error) = match load_correlation_journal(path) {
        Ok(threads) => (threads, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    republish_engine_lane_runs(&threads);
    (threads, load_error)
}

/// Refill the lane index from the correlation journal on disk.
///
/// Returns the number of lane-bound threads the journal named. The error, if
/// the journal cannot be read, is returned rather than logged: a caller that
/// asked for the restart edge explicitly needs to know it did not happen, and
/// an empty index is indistinguishable from a journal with nothing in it.
pub fn reload_engine_lane_runs_from_disk() -> anyhow::Result<usize> {
    let path = correlation_journal_path();
    let (threads, load_error) = load_journal_and_republish(&path);
    if let Some(error) = load_error {
        anyhow::bail!(
            "correlation journal at {} is unreadable: {error}",
            path.display()
        );
    }
    Ok(threads.len())
}

/// The `omega-effectd` lane run this thread belongs to, if it is one.
///
/// Returns `None` for every thread the user started themselves, which is what
/// keeps a hand-driven `codex-acp` thread disclosed as routed rather than as a
/// delegated lane run.
pub fn engine_lane_run(thread_id: ThreadId) -> Option<String> {
    match ENGINE_LANE_RUNS.lock() {
        Ok(index) => index.get(&thread_id).cloned(),
        Err(poisoned) => poisoned.into_inner().get(&thread_id).cloned(),
    }
}

#[derive(Clone)]
struct WorkspaceBinding {
    workspace_ref: String,
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
}

#[derive(Clone)]
struct HostThread {
    workspace_ref: String,
    lane: String,
    operation_ref: String,
    thread_id: ThreadId,
    conversation: Option<WeakEntity<ConversationView>>,
    turns: Vec<HostTurn>,
    revision: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostTurn {
    turn_ref: String,
    lane: String,
    account_ref: Option<String>,
    model: Option<String>,
    provider_session_ref: String,
    start_entry_index: usize,
    end_entry_index: Option<usize>,
    phase: String,
    disposition: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostTurnOutcome {
    Completed,
    Failed,
    TimedOut,
}

struct HostBridgeState {
    workspace: Option<WorkspaceBinding>,
    threads: Vec<HostThread>,
    correlation_path: PathBuf,
    load_error: Option<String>,
    sarah_conversation: Option<Arc<Mutex<SarahConversationClient>>>,
    device_bridge: Option<omega_effectd::DeviceBridgeServerHandle>,
    device_projection: Option<omega_effectd::ProjectionJournal>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CorrelationJournal {
    schema: String,
    threads: Vec<PersistedHostThread>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedHostThread {
    workspace_ref: String,
    lane: String,
    operation_ref: String,
    thread_ref: String,
    turns: Vec<HostTurn>,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolveWorkspaceParams {
    expected_workspace_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveSyncSessionParams {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateThreadParams {
    title: String,
    lane: String,
    workspace_ref: String,
    operation_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaneReadinessParams {
    lane: String,
    excluding_thread_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DispatchTurnParams {
    run_ref: String,
    workspace_ref: String,
    thread_ref: String,
    turn_ref: String,
    message: String,
    profile: Option<DispatchProfile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DispatchProfile {
    lane: Option<String>,
    account_ref: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefreshEvidenceParams {
    run_ref: String,
    thread_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InterruptTurnParams {
    thread_ref: String,
    turn_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppendSystemNoteParams {
    thread_ref: String,
    note_ref: String,
    text: String,
}

thread_local! {
    /// The live host-bridge state, so a surface outside the engine callback can
    /// reach the same threads, journal, and conversation the callback owns.
    ///
    /// omega#124. Zero base renders the pairing control but loads no workroom
    /// panel, so no Sarah host request ever arrives and the pairing runtime was
    /// never installed. The control then refused every press with "Direct phone
    /// pairing is not available on this host", which made the default mode the
    /// one mode that could not pair. This handle lets the control start the
    /// same transport the callback starts, against the same state, so the
    /// mirror a paired phone reads is the mirror this window publishes.
    static HOST_BRIDGE_STATE: RefCell<Option<Rc<RefCell<HostBridgeState>>>> =
        const { RefCell::new(None) };
}

/// Install the device-pairing runtime if no Sarah host request has installed it.
///
/// omega#124. Returns once the runtime is available, or with the reason it is
/// not. The work is idempotent: a second call with the runtime already present
/// does nothing, and the conversation and bridge are created at most once.
pub async fn ensure_device_pairing_runtime(cx: &mut AsyncApp) -> anyhow::Result<()> {
    if cx.update(|cx| omega_effectd::has_device_pairing(cx)) {
        return Ok(());
    }
    let state = HOST_BRIDGE_STATE
        .with(|slot| slot.borrow().clone())
        .ok_or_else(|| anyhow::anyhow!("the Omega engine host bridge is not installed"))?;
    if state.borrow().sarah_conversation.is_some() {
        return Ok(());
    }
    let conversation =
        production_sarah_conversation().map_err(|error| anyhow::anyhow!("{}", error.message))?;
    let device_bridge = start_device_bridge(&conversation, &state, cx)
        .map_err(|error| anyhow::anyhow!("{}", error.message))?;
    let Some((handle, engine, endpoint, host_public_key_hex, generation, scopes, journal)) =
        device_bridge
    else {
        anyhow::bail!("this host advertises no direct device endpoint");
    };
    cx.update(|cx| {
        omega_effectd::configure_device_pairing(
            engine,
            endpoint,
            host_public_key_hex,
            generation,
            scopes,
            cx,
        );
    });
    let conversation = Arc::new(Mutex::new(conversation));
    {
        let mut state = state.borrow_mut();
        state.sarah_conversation = Some(conversation.clone());
        state.device_bridge = Some(handle);
        state.device_projection = Some(journal);
    }
    start_issue31_agent_command_pump(conversation, state.clone(), cx);
    start_panel_thread_projection(state, cx);
    Ok(())
}

pub fn omega_effectd_host_handler(cx: &App) -> OmegaEffectdHostHandler {
    let async_cx = cx.to_async();
    let openagents_session = omega_effectd::openagents_session(cx);
    let correlation_path = correlation_journal_path();
    // OMEGA-DELTA-0021. This is the restart edge: the lane index is empty in a
    // freshly started process, and the journal on disk is what refills it, so a
    // thread resumed after a restart still discloses the lane that owns it.
    let (threads, load_error) = load_journal_and_republish(&correlation_path);
    let state = Rc::new(RefCell::new(HostBridgeState {
        workspace: None,
        threads,
        correlation_path,
        load_error,
        sarah_conversation: None,
        device_bridge: None,
        device_projection: None,
    }));
    HOST_BRIDGE_STATE.with(|slot| {
        *slot.borrow_mut() = Some(state.clone());
    });
    Rc::new(move |request| {
        let async_cx = async_cx.clone();
        let state = state.clone();
        let openagents_session = openagents_session.clone();
        Box::pin(async move { handle_request(request, state, openagents_session, async_cx).await })
    })
}

/// OMEGA-DELTA-0167. A shipped app is launched from Finder, which inherits no
/// shell environment, so a value the pairing runtime refuses to start without
/// must not live in an environment variable: it is then guaranteed absent
/// exactly where it is needed. omega#124 shipped with six such variables,
/// which made "Pair phone" a control that could only fail in an installed
/// build. Everything below is a product fact rather than a per-machine fact,
/// so it is compiled in, and each environment variable survives only as a
/// development override.
const DEFAULT_NOSTR_RELAY_URL: &str = "wss://relay.openagents.com";

/// Sarah's production Nostr bridge identity. A public key is not a secret —
/// the same value is what the openagents deployment publishes as
/// `SARAH_NOSTR_OWNER_PUBKEY`, and a device must already know it to verify
/// what it reads.
const DEFAULT_SARAH_PUBLIC_KEY_HEX: &str =
    "bcf86577b45042c960c99fe4ac1380a3ef0565ccbdd5c81e3f20f0919fe4fd14";

/// The bridge projection remains read-only. A paired owner phone also receives
/// the narrow `send_message` scope so its separately signed command intent can
/// enqueue or steer an existing Omega agent thread through the audited host
/// command pump.
const DEFAULT_DEVICE_SCOPES: &[omega_effectd::Issue31PairingScope] = &[
    omega_effectd::Issue31PairingScope::ObserveIssue31,
    omega_effectd::Issue31PairingScope::SendMessage,
];

/// Fallback when Tailscale is absent. A phone cannot dial `localhost` — that
/// is only useful for a simulator on the same machine. When Tailscale is up
/// the live MagicDNS name and CGNAT bind replace these (see
/// [`live_tailnet_endpoint`]).
const DEFAULT_DEVICE_BRIDGE_MAGIC_DNS: &str = "localhost";
const DEFAULT_DEVICE_BRIDGE_PORT: u16 = 4317;
const DEFAULT_DEVICE_BRIDGE_BIND_ADDRESS: &str = "127.0.0.1";

/// What a live Tailscale status contributes to the pairing endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveTailnetEndpoint {
    /// MagicDNS name without a trailing dot — e.g. `mac.tailnet.ts.net`.
    magic_dns_name: String,
    /// First Tailscale IPv4 in 100.64.0.0/10. The bridge binds here so a phone
    /// on the same tailnet can reach it; loopback would never answer a phone.
    bind_address: String,
}

/// How an override is read. Production reads the environment; a test supplies
/// its own map, so the defaults can be proven without mutating process state
/// that every other test in the binary shares.
type PairingOverrides<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Read a development override from the environment.
fn pairing_override(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Read an override through the rule every override obeys: a blank value
/// counts as unset. An `export` left empty in a launch script would otherwise
/// erase a product default and put the app back where it started, and this
/// belongs here rather than in the environment reader so that every lookup —
/// including a future settings-backed one — inherits it.
fn resolved_override(overrides: PairingOverrides<'_>, name: &str) -> Option<String> {
    overrides(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn comma_separated(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

fn resolve_relay_urls(overrides: PairingOverrides<'_>) -> Vec<String> {
    resolved_override(overrides, "OPENAGENTS_OMEGA_NOSTR_RELAYS")
        .map(|value| comma_separated(&value))
        .filter(|relay_urls| !relay_urls.is_empty())
        .unwrap_or_else(|| vec![DEFAULT_NOSTR_RELAY_URL.to_owned()])
}

fn resolve_sarah_public_key_hex(overrides: PairingOverrides<'_>) -> String {
    resolved_override(overrides, "OPENAGENTS_OMEGA_SARAH_PUBLIC_KEY_HEX")
        .unwrap_or_else(|| DEFAULT_SARAH_PUBLIC_KEY_HEX.to_owned())
}

/// The devices already admitted to the relay lane.
///
/// This is state a pairing produces, not configuration an owner types. A fresh
/// install legitimately admits no device, and the QR flow in
/// `issue_direct_pairing_grant` is what admits the first one — it writes the
/// device into its own admitted set only after that device proves possession
/// of its key. Empty is therefore both honest and the safe end: it admits
/// nobody who has not paired.
fn resolve_admitted_device_public_key_hexes(overrides: PairingOverrides<'_>) -> Vec<String> {
    resolved_override(overrides, "OPENAGENTS_OMEGA_NOSTR_DEVICE_PUBLIC_KEYS")
        .map(|value| comma_separated(&value))
        .unwrap_or_default()
}

fn resolve_approved_device_scopes(
    overrides: PairingOverrides<'_>,
) -> Result<Vec<omega_effectd::Issue31PairingScope>, HostResponseError> {
    match resolved_override(overrides, "OPENAGENTS_OMEGA_NOSTR_DEVICE_SCOPES") {
        Some(value) => comma_separated(&value)
            .iter()
            .map(|scope| omega_effectd::Issue31PairingScope::parse(scope))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                unavailable(format!(
                    "The device mirror scope policy is invalid: {error}"
                ))
            }),
        None => Ok(DEFAULT_DEVICE_SCOPES.to_vec()),
    }
}

/// Ask the local Tailscale daemon who we are on the tailnet.
///
/// The QR a phone scans has to name a host the phone can dial. `localhost`
/// and `127.0.0.1` only ever answer on the machine that printed the code, so
/// a real phone always fails with "Could not connect to ws://localhost:4317".
/// When Tailscale is up we take the MagicDNS name and the CGNAT IPv4 from
/// `tailscale status --json` and use those as the product default — no owner
/// configuration, no environment variables. When Tailscale is down or the
/// binary is missing we fall back to loopback so a same-machine simulator
/// still pairs.
fn discover_live_tailnet() -> Option<LiveTailnetEndpoint> {
    let output = smol::block_on(
        smol::process::Command::new("tailscale")
            .args(["status", "--json"])
            .output(),
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    live_tailnet_from_status_json(&output.stdout)
}

/// Parse the subset of `tailscale status --json` the pairing defaults need.
/// Kept pure so a unit test can pin the shape without a live daemon.
fn live_tailnet_from_status_json(bytes: &[u8]) -> Option<LiveTailnetEndpoint> {
    let status: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let self_node = status.get("Self")?;
    let magic_dns_name = self_node
        .get("DNSName")
        .and_then(Value::as_str)
        .map(|name| name.trim_end_matches('.').to_owned())
        .filter(|name| !name.is_empty() && !name.contains("://") && !name.contains('/'))?;
    let bind_address = self_node
        .get("TailscaleIPs")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .find(|ip| {
            ip.parse::<std::net::Ipv4Addr>().ok().is_some_and(|addr| {
                let bits = u32::from(addr);
                bits & 0xffc0_0000 == u32::from(std::net::Ipv4Addr::new(100, 64, 0, 0))
            })
        })?
        .to_owned();
    Some(LiveTailnetEndpoint {
        magic_dns_name,
        bind_address,
    })
}

/// Process-wide live Tailscale view. Discovered once; a missing daemon is
/// remembered as `None` so a crash-loop of `tailscale status` is not the
/// cost of every Pair-phone click.
fn live_tailnet_endpoint() -> Option<&'static LiveTailnetEndpoint> {
    static LIVE: LazyLock<Option<LiveTailnetEndpoint>> = LazyLock::new(discover_live_tailnet);
    LIVE.as_ref()
}

/// Loopback when no tailnet is available; the live MagicDNS name when one is.
fn default_magic_dns_name(live: Option<&LiveTailnetEndpoint>) -> String {
    live.map(|endpoint| endpoint.magic_dns_name.clone())
        .unwrap_or_else(|| DEFAULT_DEVICE_BRIDGE_MAGIC_DNS.to_owned())
}

/// Loopback when no tailnet is available; the live CGNAT IPv4 when one is.
fn default_bind_address(live: Option<&LiveTailnetEndpoint>) -> String {
    live.map(|endpoint| endpoint.bind_address.clone())
        .unwrap_or_else(|| DEFAULT_DEVICE_BRIDGE_BIND_ADDRESS.to_owned())
}

fn resolve_bind_address(overrides: PairingOverrides<'_>) -> String {
    resolve_bind_address_with(overrides, live_tailnet_endpoint())
}

fn resolve_bind_address_with(
    overrides: PairingOverrides<'_>,
    live: Option<&LiveTailnetEndpoint>,
) -> String {
    resolved_override(overrides, "OPENAGENTS_OMEGA_DEVICE_BRIDGE_BIND_ADDRESS")
        .unwrap_or_else(|| default_bind_address(live))
}

/// Where the direct device bridge advertises itself.
///
/// The MagicDNS name and the port describe one endpoint, so supplying one
/// without the other yields a host a phone can discover and cannot reach. That
/// stays refused, but the refusal now names which half is missing and what
/// unsetting both restores. With no overrides the live tailnet (when present)
/// is preferred over loopback — a phone cannot dial `localhost`.
fn resolve_direct_device_endpoint(
    overrides: PairingOverrides<'_>,
) -> Result<omega_effectd::Issue31DirectEndpoint, HostResponseError> {
    resolve_direct_device_endpoint_with(overrides, live_tailnet_endpoint())
}

fn resolve_direct_device_endpoint_with(
    overrides: PairingOverrides<'_>,
    live: Option<&LiveTailnetEndpoint>,
) -> Result<omega_effectd::Issue31DirectEndpoint, HostResponseError> {
    let magic_dns_override =
        resolved_override(overrides, "OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS");
    let port_override = resolved_override(overrides, "OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT");
    let built_in = format!(
        "{}:{}",
        default_magic_dns_name(live),
        DEFAULT_DEVICE_BRIDGE_PORT
    );
    let (magic_dns_name, port) = match (magic_dns_override, port_override) {
        (None, None) => (default_magic_dns_name(live), DEFAULT_DEVICE_BRIDGE_PORT),
        (Some(magic_dns_name), Some(port)) => {
            let parsed = port.parse::<u16>().ok().filter(|port| *port != 0).ok_or_else(|| {
                unavailable(format!(
                    "OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT is \"{port}\", which is not a TCP port \
                     between 1 and 65535. Unset it and \
                     OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS to use the built-in {built_in}."
                ))
            })?;
            (magic_dns_name, parsed)
        }
        (Some(_), None) => {
            return Err(unavailable(format!(
                "OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS is set but \
                 OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT is not, so this host would advertise a name \
                 with no port. Set both, or unset both to use the built-in {built_in}."
            )));
        }
        (None, Some(_)) => {
            return Err(unavailable(format!(
                "OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT is set but \
                 OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS is not, so this host would advertise a \
                 port with no name. Set both, or unset both to use the built-in {built_in}."
            )));
        }
    };
    Ok(omega_effectd::Issue31DirectEndpoint {
        magic_dns_name,
        port,
        protocol: omega_effectd::DEVICE_BRIDGE_PROTOCOL.into(),
    })
}

fn production_sarah_conversation() -> Result<SarahConversationClient, HostResponseError> {
    let relay_urls = resolve_relay_urls(&pairing_override);
    let sarah_public_key_hex = resolve_sarah_public_key_hex(&pairing_override);
    let admitted_device_public_key_hexes =
        resolve_admitted_device_public_key_hexes(&pairing_override);
    let approved_device_scopes = resolve_approved_device_scopes(&pairing_override)?;
    let community_group_ids = resolved_override(
        &pairing_override,
        "OPENAGENTS_OMEGA_NOSTR_COMMUNITY_GROUP_IDS",
    )
    .map(|value| comma_separated(&value))
    .unwrap_or_default();
    let community_public_key_hexes = resolved_override(
        &pairing_override,
        "OPENAGENTS_OMEGA_NOSTR_COMMUNITY_PUBLIC_KEYS",
    )
    .map(|value| comma_separated(&value))
    .unwrap_or_default();
    let identity_service = Arc::new(omega_identity::IdentityService::system(
        *app_identity::CHANNEL,
    ));
    // Provision rather than inspect. Startup now provisions the identity in
    // the background (omega#164), but this path keeps its own provisioning:
    // a person who clicks "Pair phone" on a profile whose launch-time
    // provisioning was interrupted must not be told custody "is not ready"
    // with nothing on screen that could make it ready.
    // The states `provision_unattended` refuses (`Lost`, `Conflict`, reset)
    // still refuse here, by name, because replacing an identity unattended is
    // the silent pick omega#110 forbids.
    let receipt_ref = omega_identity::ReceiptRef::new("omega-device-pairing-provision-v1")
        .map_err(|error| unavailable(format!("Omega identity receipt ref is invalid: {error}")))?;
    let custody = identity_service
        .provision_unattended(receipt_ref)
        .map_err(|error| unavailable(format!("Omega identity custody is unavailable: {error}")))?;
    let owner_public_key_hex = custody
        .identity
        .map(|identity| identity.public_key_hex().as_str().to_string())
        .ok_or_else(|| unavailable("Omega identity custody is not ready."))?;
    let conversation_digest = resolved_override(
        &pairing_override,
        "OPENAGENTS_OMEGA_SARAH_CONVERSATION_DIGEST",
    )
    .unwrap_or_else(|| owner_public_key_hex.chars().take(24).collect());
    // Always one endpoint. This used to default to none, and because the
    // direct loopback bridge is built here, a host with no relay configuration
    // also advertised no direct endpoint — so QR pairing, which needs no relay
    // at all, died of a missing relay variable.
    let direct_endpoints = vec![resolve_direct_device_endpoint(&pairing_override)?];
    let config = SarahConversationConfig {
        generation: 1,
        conversation_digest,
        identity: ConversationIdentity {
            owner_public_key_hex,
            sarah_public_key_hex,
            account_label: None,
            binding_state: omega_effectd::BindingState::Unbound,
        },
        relay_url: relay_urls.first().cloned(),
        direct_endpoints,
        admitted_device_public_key_hexes,
        approved_device_scopes,
        community_group_ids,
        community_public_key_hexes,
    };
    let mut conversation =
        SarahConversationClient::new_production(config, relay_urls, identity_service).map_err(
            |error| unavailable(format!("Sarah Nostr transport is unavailable: {error}")),
        )?;
    // omega#49: the host pump publishes the omega#47 snapshot and its Full Auto
    // detail to every admitted device. Without this the two documents are built
    // by the desktop panel and never leave the machine, and a paired phone
    // reports `no_host_projection` for a host that is running work.
    conversation.set_issue31_host_projection_source(full_auto_ui::issue31_host_projection_source());
    // omega#91: how the host reads its own provider accounts when it decides
    // which one a connection handoff binds to. Without it no handoff can bind,
    // and every one the phone opens runs to its deadline and expires.
    conversation.set_issue31_provider_roster_source(full_auto_ui::issue31_provider_roster_source());
    Ok(conversation)
}

async fn sarah_request(
    request: HostRequestFrame,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let method = match request.method {
        HostMethod::SarahSessionStatus => SARAH_METHOD_SESSION_STATUS,
        HostMethod::SarahBootstrap => SARAH_METHOD_BOOTSTRAP,
        HostMethod::SarahRoomSnapshot => SARAH_METHOD_ROOM_SNAPSHOT,
        HostMethod::SarahSendMessage => SARAH_METHOD_SEND_MESSAGE,
        HostMethod::SarahInterruptTurn => SARAH_METHOD_INTERRUPT_TURN,
        HostMethod::SarahDeviceGrants => SARAH_METHOD_DEVICE_GRANTS,
        HostMethod::SarahRenewDeviceGrant => SARAH_METHOD_RENEW_DEVICE_GRANT,
        HostMethod::SarahRevokeDeviceGrant => SARAH_METHOD_REVOKE_DEVICE_GRANT,
        _ => return Err(unsupported("Unknown Sarah host method.")),
    };
    let conversation = state.borrow().sarah_conversation.clone();
    let conversation = match conversation {
        Some(conversation) => conversation,
        None => match production_sarah_conversation() {
            Ok(conversation) => {
                let device_bridge = start_device_bridge(&conversation, state, cx)?;
                if let Some((_, engine, endpoint, host_public_key_hex, generation, scopes, _)) =
                    device_bridge.as_ref()
                {
                    let engine = engine.clone();
                    let endpoint = endpoint.clone();
                    let host_public_key_hex = host_public_key_hex.clone();
                    let scopes = scopes.clone();
                    let generation = *generation;
                    cx.update(|cx| {
                        omega_effectd::configure_device_pairing(
                            engine,
                            endpoint,
                            host_public_key_hex,
                            generation,
                            scopes,
                            cx,
                        );
                    });
                }
                let conversation = Arc::new(Mutex::new(conversation));
                {
                    let mut state = state.borrow_mut();
                    state.sarah_conversation = Some(conversation.clone());
                    if let Some((handle, _, _, _, _, _, journal)) = device_bridge {
                        state.device_bridge = Some(handle);
                        state.device_projection = Some(journal);
                    }
                }
                start_issue31_agent_command_pump(conversation.clone(), state.clone(), cx);
                conversation
            }
            Err(error) => return Err(error),
        },
    };
    let params = request.params;
    let generation = request.generation;
    cx.update(|cx| {
        refresh_device_generation(state, generation, cx)
            .map_err(|error| unavailable(format!("Device mirror refresh failed: {error:#}")))
    })?;
    cx.background_spawn(async move {
        let mut conversation = conversation
            .lock()
            .map_err(|_| unavailable("Sarah Nostr transport state is unavailable."))?;
        conversation
            .synchronize_process_generation(generation)
            .map_err(sarah_host_error)?;
        conversation
            .handle_request(method, generation, Some(&params))
            .map_err(sarah_host_error)
    })
    .await
}

/// Project the Agent Panel's own threads into the device mirror.
///
/// omega#124. Every existing publisher writes engine-lane threads the host
/// bridge tracks, so a paired phone saw an empty feed while the person typed in
/// a thread on the desktop. The mirror's stated contract is the desktop's
/// active and recent threads, so the panel's threads belong in it. This reads
/// the panel and publishes an upsert whenever a thread's projection changes.
fn start_panel_thread_projection(state: Rc<RefCell<HostBridgeState>>, cx: &mut AsyncApp) {
    cx.spawn(async move |cx| {
        let mut published: StdHashMap<String, omega_effectd::MirrorThread> = StdHashMap::new();
        loop {
            cx.background_executor()
                .timer(PANEL_PROJECTION_INTERVAL)
                .await;
            let Some(journal) = state.borrow().device_projection.clone() else {
                continue;
            };
            let projected = cx.update(|cx| panel_mirror_threads(&state, cx));
            for thread in projected {
                // The stamp now comes from the thread metadata store rather
                // than from the poll clock, so an unchanged thread projects
                // identically pass after pass and this comparison stops the
                // mirror republishing every thread several times a second.
                if published
                    .get(&thread.thread_ref)
                    .is_some_and(|previous| previous == &thread)
                {
                    continue;
                }
                if journal
                    .publish(omega_effectd::MirrorChange::ThreadUpsert {
                        thread: thread.clone(),
                    })
                    .is_ok()
                {
                    published.insert(thread.thread_ref.clone(), thread);
                }
            }
        }
    })
    .detach();
}

/// The panel's loaded conversations, projected for the mirror.
fn panel_mirror_threads(
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut App,
) -> Vec<omega_effectd::MirrorThread> {
    // The engine binds a workspace only when it dispatches a run. A person who
    // paired a phone and typed in a thread has no such binding, so the mirror
    // finds the open workspace itself and falls back to the binding when the
    // engine did establish one (omega#124).
    let bound = state.borrow().workspace.clone();
    let located = match bound {
        Some(binding) => binding.workspace.upgrade().map(|w| (binding.window, w)),
        None => AppState::global(cx)
            .workspace_store
            .read(cx)
            .workspaces_with_windows()
            .find_map(|(window, workspace)| Some((window, workspace.upgrade()?))),
    };
    let Some((window, workspace)) = located else {
        return Vec::new();
    };
    let projected_at = current_unix_millis();
    window
        .update(cx, |_root, _window, cx| {
            let Some(panel) = workspace.read(cx).panel::<AgentPanel>(cx) else {
                return Vec::new();
            };
            panel
                .read(cx)
                .conversation_views()
                .into_iter()
                .map(|conversation| device_loaded_thread(&conversation, projected_at, cx))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn start_issue31_agent_command_pump(
    conversation: Arc<Mutex<SarahConversationClient>>,
    state: Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(ISSUE31_AGENT_COMMAND_POLL_INTERVAL)
                .await;
            let conversation_for_sync = conversation.clone();
            let pending = cx
                .background_spawn(async move {
                    let mut conversation = conversation_for_sync.lock().map_err(|_| {
                        anyhow::anyhow!("Sarah Nostr transport state is unavailable")
                    })?;
                    conversation.sync_issue31_host()?;
                    anyhow::Ok(conversation.pending_agent_thread_commands())
                })
                .await;
            let pending = match pending {
                Ok(pending) => pending,
                Err(error) => {
                    log::warn!("Device mirror command sync failed: {error:#}");
                    continue;
                }
            };
            for command in pending {
                if command.admission_state
                    == omega_effectd::Issue31AgentThreadAdmissionState::Pending
                {
                    let conversation_for_admission = conversation.clone();
                    let idempotency_ref = command.idempotency_ref.clone();
                    let admission_persisted = cx
                        .background_spawn(async move {
                            let mut conversation =
                                conversation_for_admission.lock().map_err(|_| {
                                    anyhow::anyhow!("Sarah Nostr transport state is unavailable")
                                })?;
                            conversation
                                .mark_agent_thread_command_admitted(&idempotency_ref)
                                .map_err(anyhow::Error::from)
                        })
                        .await;
                    match admission_persisted {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(error) => {
                            log::error!(
                                "Device mirror command {} could not reserve durable admission: {error:#}",
                                command.idempotency_ref
                            );
                            continue;
                        }
                    }

                    let admitted = cx.update(|cx| {
                        admit_issue31_agent_thread_command(&state, &command, cx)
                    });
                    if let Err(error) = admitted {
                        let conversation_for_retry = conversation.clone();
                        let idempotency_ref = command.idempotency_ref.clone();
                        let retry_result = cx
                            .background_spawn(async move {
                                let mut conversation =
                                    conversation_for_retry.lock().map_err(|_| {
                                        anyhow::anyhow!(
                                            "Sarah Nostr transport state is unavailable"
                                        )
                                    })?;
                                conversation
                                    .mark_agent_thread_command_pending(&idempotency_ref)?;
                                anyhow::Ok(())
                            })
                            .await;
                        if let Err(retry_error) = retry_result {
                            log::error!(
                                "Device mirror command {} was not admitted and its durable retry could not be restored: {retry_error:#}",
                                command.idempotency_ref
                            );
                        } else {
                            log::warn!(
                                "Device mirror command {} is waiting for its thread: {error:#}",
                                command.idempotency_ref
                            );
                        }
                        continue;
                    }
                }
                let conversation_for_completion = conversation.clone();
                let idempotency_ref = command.idempotency_ref.clone();
                if let Err(error) = cx
                    .background_spawn(async move {
                        let mut conversation =
                            conversation_for_completion.lock().map_err(|_| {
                                anyhow::anyhow!("Sarah Nostr transport state is unavailable")
                            })?;
                        conversation.complete_agent_thread_command(&idempotency_ref)?;
                        anyhow::Ok(())
                    })
                    .await
                {
                    log::error!(
                        "Device mirror command {} was admitted but not durably completed: {error:#}",
                        command.idempotency_ref
                    );
                }
            }
        }
    })
    .detach();
}

fn admit_issue31_agent_thread_command(
    state: &Rc<RefCell<HostBridgeState>>,
    command: &omega_effectd::Issue31PendingAgentThreadCommand,
    cx: &mut App,
) -> anyhow::Result<()> {
    let thread_id = ThreadId::from_key_string(&command.thread_ref)?;
    let binding = state
        .borrow()
        .workspace
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no Omega workspace is bound"))?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| anyhow::anyhow!("the bound Omega workspace was closed"))?;
    binding
        .window
        .update(cx, |_root, window, cx| {
            let panel = workspace
                .read(cx)
                .panel::<AgentPanel>(cx)
                .ok_or_else(|| anyhow::anyhow!("the Agent panel is unavailable"))?;
            let conversation = panel
                .read(cx)
                .conversation_view_for_id(&thread_id, cx)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("the target Agent thread is not loaded"))?;
            let thread_view = conversation
                .read(cx)
                .root_thread_view()
                .ok_or_else(|| anyhow::anyhow!("the target Agent thread is still connecting"))?;
            thread_view
                .update(cx, |thread_view, cx| {
                    thread_view.admit_issue31_agent_thread_message(
                        &command.text,
                        command.disposition,
                        window,
                        cx,
                    )
                })
                .map_err(anyhow::Error::msg)?;
            if let Err(error) = publish_device_thread(state, &command.thread_ref, cx) {
                log::warn!(
                    "Device mirror command {} was admitted but its mirror refresh failed: {error:#}",
                    command.idempotency_ref
                );
            }
            anyhow::Ok(())
        })
        .map_err(|error| anyhow::anyhow!("the Omega workspace window is unavailable: {error}"))?
}

type DeviceBridgeStartup = (
    omega_effectd::DeviceBridgeServerHandle,
    omega_effectd::DevicePairingEngine,
    omega_effectd::Issue31DirectEndpoint,
    String,
    u64,
    Vec<omega_effectd::Issue31PairingScope>,
    omega_effectd::ProjectionJournal,
);

fn start_device_bridge(
    conversation: &SarahConversationClient,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Option<DeviceBridgeStartup>, HostResponseError> {
    let Some((engine, endpoint, host_public_key_hex, generation, scopes)) =
        conversation.device_pairing_runtime()
    else {
        return Ok(None);
    };
    let bind_address = resolve_bind_address(&pairing_override);
    let host = omega_effectd::BridgeBindHost::new(&bind_address).map_err(|error| {
        unavailable(format!(
            "OPENAGENTS_OMEGA_DEVICE_BRIDGE_BIND_ADDRESS is \"{bind_address}\": {error}. Unset it \
             to bind the built-in {DEFAULT_DEVICE_BRIDGE_BIND_ADDRESS}."
        ))
    })?;
    let config = omega_effectd::DeviceBridgeServerConfig {
        host,
        port: endpoint.port,
        heartbeat_interval: Duration::from_secs(5),
    };
    let snapshot = cx
        .update(|cx| device_mirror_snapshot(&state.borrow(), generation, cx))
        .map_err(|error| unavailable(format!("Device mirror snapshot failed: {error:#}")))?;
    let journal = omega_effectd::ProjectionJournal::new(snapshot);
    let handle =
        omega_effectd::start_pairable_device_bridge_server(config, engine.clone(), journal.clone())
            .map_err(|error| {
                unavailable(format!("Omega device bridge failed to start: {error}"))
            })?;
    Ok(Some((
        handle,
        engine,
        endpoint,
        host_public_key_hex,
        generation,
        scopes,
        journal,
    )))
}

fn device_mirror_snapshot(
    state: &HostBridgeState,
    generation: u64,
    cx: &App,
) -> anyhow::Result<omega_effectd::MirrorSnapshot> {
    let projected_at = current_unix_millis();
    let reading = full_auto_ui::issue31_device_mirror_reading();
    let mut runs = reading
        .as_ref()
        .map_or_else(Vec::new, |reading| reading.runs.clone());
    for thread in &state.threads {
        if !runs.iter().any(|run| run.run_ref == thread.operation_ref) {
            runs.push(device_mirror_run(thread, projected_at));
        }
    }
    runs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.run_ref.cmp(&right.run_ref))
    });
    runs.truncate(MAX_DEVICE_RUNS);
    let mut threads = state
        .threads
        .iter()
        .map(|thread| device_mirror_thread(thread, projected_at, cx))
        .collect::<Vec<_>>();
    let mut projected_thread_refs = threads
        .iter()
        .map(|thread| thread.thread_ref.clone())
        .collect::<HashSet<_>>();
    if let Some(workspace) = state
        .workspace
        .as_ref()
        .and_then(|binding| binding.workspace.upgrade())
        && let Some(panel) = workspace.read(cx).panel::<AgentPanel>(cx)
    {
        let panel = panel.read(cx);
        if let Some(conversation) = panel.active_conversation_view() {
            let projected = device_loaded_thread(conversation, projected_at, cx);
            if projected_thread_refs.insert(projected.thread_ref.clone()) {
                threads.push(projected);
            }
        }
        for conversation in panel.retained_threads().values() {
            let projected = device_loaded_thread(conversation, projected_at, cx);
            if projected_thread_refs.insert(projected.thread_ref.clone()) {
                threads.push(projected);
            }
        }
    }
    threads.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.thread_ref.cmp(&right.thread_ref))
    });
    threads.truncate(MAX_DEVICE_THREADS);
    bound_device_mirror_snapshot(omega_effectd::MirrorSnapshot {
        desktop_name: "Local Omega".to_string(),
        generation,
        sequence: 0,
        threads,
        runs,
        health: omega_effectd::MirrorHealth {
            engine_up: true,
            engine_generation: reading
                .as_ref()
                .map_or(generation, |reading| reading.engine_generation),
            lane_ready: reading.as_ref().is_some_and(|reading| reading.lane_ready),
            observed_at: projected_at,
        },
        projected_at,
    })
}

fn bound_device_mirror_snapshot(
    mut snapshot: omega_effectd::MirrorSnapshot,
) -> anyhow::Result<omega_effectd::MirrorSnapshot> {
    let frame_budget =
        omega_effectd::MAX_FRAME_BYTES.saturating_sub(DEVICE_SNAPSHOT_FRAME_RESERVE_BYTES);
    loop {
        if serde_json::to_vec(&snapshot)?.len() <= frame_budget {
            return Ok(snapshot);
        }
        if let Some((thread_index, _)) = snapshot
            .threads
            .iter()
            .enumerate()
            .filter(|(_, thread)| !thread.transcript.is_empty())
            .max_by_key(|(_, thread)| thread.transcript.len())
        {
            snapshot.threads[thread_index].transcript.remove(0);
            continue;
        }
        if snapshot.runs.pop().is_some() {
            continue;
        }
        if snapshot.threads.pop().is_some() {
            continue;
        }
        anyhow::bail!("the empty device mirror snapshot exceeds its transport frame budget");
    }
}

fn device_mirror_thread(
    thread: &HostThread,
    projected_at: u64,
    cx: &App,
) -> omega_effectd::MirrorThread {
    let (title, executor, state, transcript) = thread
        .conversation
        .as_ref()
        .and_then(WeakEntity::upgrade)
        .and_then(|conversation| {
            let thread_view = conversation.read(cx).root_thread_view()?;
            let error_text = thread_view.read(cx).device_mirror_error_text(cx);
            let disclosure = thread_view.read(cx).executor_disclosure(cx);
            let root_thread = conversation.read(cx).root_thread(cx)?;
            let root_thread = root_thread.read(cx);
            let title = root_thread
                .title_or_first_user_message(cx)
                .map_or_else(|| thread.operation_ref.clone(), |title| title.to_string());
            let state = device_thread_state(root_thread.status(), thread);
            let thread_ref = thread.thread_id.to_key_string();
            let mut transcript =
                device_transcript(&thread_ref, root_thread.entries(), projected_at, cx);
            append_device_error_message(
                &mut transcript,
                &thread_ref,
                error_text.as_deref(),
                projected_at,
            );
            Some((
                device_public_label(&title, &thread.operation_ref),
                device_executor_disclosure(&disclosure),
                state,
                transcript,
            ))
        })
        .unwrap_or_else(|| {
            (
                device_public_label(&thread.operation_ref, "Full Auto run"),
                fallback_device_executor_disclosure(thread),
                device_thread_state(ThreadStatus::Idle, thread),
                Vec::new(),
            )
        });
    omega_effectd::MirrorThread {
        thread_ref: thread.thread_id.to_key_string(),
        title,
        executor,
        state,
        transcript,
        updated_at: thread
            .turns
            .last()
            .map_or(projected_at, |turn| timestamp_millis(&turn.updated_at)),
    }
}

fn device_loaded_thread(
    conversation: &Entity<ConversationView>,
    projected_at: u64,
    cx: &App,
) -> omega_effectd::MirrorThread {
    let conversation = conversation.read(cx);
    let thread_ref = conversation.thread_id.to_key_string();
    let projected_at = device_thread_updated_at(conversation, projected_at, cx);
    let Some(thread_view) = conversation.root_thread_view() else {
        return omega_effectd::MirrorThread {
            thread_ref,
            title: "Omega thread".to_string(),
            executor: omega_effectd::ExecutorDisclosure {
                executor_id: "omega-agent".to_string(),
                executor_name: "Omega".to_string(),
                model_id: None,
                model_name: None,
            },
            state: omega_effectd::ThreadState::Idle,
            transcript: Vec::new(),
            updated_at: projected_at,
        };
    };
    let error_text = thread_view.read(cx).device_mirror_error_text(cx);
    let disclosure = thread_view.read(cx).executor_disclosure(cx);
    let Some(thread) = conversation.root_thread(cx) else {
        return omega_effectd::MirrorThread {
            thread_ref,
            title: "Omega thread".to_string(),
            executor: device_executor_disclosure(&disclosure),
            state: omega_effectd::ThreadState::Idle,
            transcript: Vec::new(),
            updated_at: projected_at,
        };
    };
    let thread = thread.read(cx);
    let mut transcript = device_transcript(&thread_ref, thread.entries(), projected_at, cx);
    append_device_error_message(
        &mut transcript,
        &thread_ref,
        error_text.as_deref(),
        projected_at,
    );
    omega_effectd::MirrorThread {
        thread_ref: thread_ref.clone(),
        title: thread.title_or_first_user_message(cx).map_or_else(
            || "Omega thread".to_string(),
            |title| device_public_label(&title, "Omega thread"),
        ),
        executor: device_executor_disclosure(&disclosure),
        state: if thread.status() == ThreadStatus::Generating {
            omega_effectd::ThreadState::Running
        } else if thread.had_error() || error_text.is_some() {
            omega_effectd::ThreadState::Failed
        } else {
            omega_effectd::ThreadState::Idle
        },
        transcript,
        updated_at: projected_at,
    }
}

/// Put the desktop callout's reason into the transcript the phone reads.
///
/// Thread failures used to project only `state: failed` with no body: the
/// phone's subtitle said "failed" and the thread view showed the person's
/// message alone, while the desktop held the full callout. Appending a
/// system turn with the same public-safe sentence means both surfaces show
/// the same reason without a second protocol field.
fn append_device_error_message(
    transcript: &mut Vec<omega_effectd::MirrorMessage>,
    thread_ref: &str,
    error_text: Option<&str>,
    projected_at: u64,
) {
    let Some(error_text) = error_text.map(str::trim).filter(|text| !text.is_empty()) else {
        return;
    };
    if !full_auto_ui::issue31_device_mirror_text_is_safe(error_text) {
        return;
    }
    let Some(text) =
        full_auto_ui::issue31_device_mirror_text(error_text, MAX_DEVICE_TRANSCRIPT_TEXT_BYTES)
    else {
        return;
    };
    // Replace a previous error turn rather than stacking one per refresh.
    if let Some(last) = transcript.last_mut()
        && last.role == omega_effectd::MessageRole::System
        && last.message_ref.ends_with(".error")
    {
        last.text = text;
        last.created_at = projected_at;
        return;
    }
    transcript.push(omega_effectd::MirrorMessage {
        message_ref: format!("{thread_ref}.error"),
        role: omega_effectd::MessageRole::System,
        text,
        created_at: projected_at,
    });
}

/// Remove the speaker heading that Zed's Markdown export writes.
///
/// `AgentThreadEntry::to_markdown` opens every entry with `## User` or
/// `## Assistant`, because a flat Markdown transcript has no other place to
/// name the speaker. The mirror carries the speaker in the typed `role` field
/// and the phone renders it as its own label, so the heading arrives as a
/// duplicate title above the same turn.
fn without_role_heading(text: &str) -> &str {
    const HEADINGS: [&str; 3] = ["## User (checkpoint)", "## Assistant", "## User"];
    for heading in HEADINGS {
        if let Some(rest) = text.strip_prefix(heading) {
            return rest.trim_start_matches(['\r', '\n']);
        }
    }
    text
}

/// When this thread last did anything, as the desktop itself knows it.
///
/// The projection used to stamp every thread with the time it happened to run,
/// so on a phone all threads read "now" and their order was arbitrary. The
/// thread metadata store is the same record the desktop sidebar dates its own
/// list from, so both surfaces now agree. A thread the store has never seen —
/// a brand new one — falls back to the projection time.
fn device_thread_updated_at(conversation: &ConversationView, projected_at: u64, cx: &App) -> u64 {
    let Some(store) = ThreadMetadataStore::try_global(cx) else {
        return projected_at;
    };
    store
        .read(cx)
        .entry(conversation.thread_id)
        .and_then(|entry| u64::try_from(entry.updated_at.timestamp_millis()).ok())
        .unwrap_or(projected_at)
}

/// One public-safe line for a tool call.
///
/// The label is what the desktop already shows in its own header, and a
/// delegation names the harness there. The state is what a person watching
/// from a phone actually wants. Arguments, diffs, file contents, and command
/// output stay on the desktop.
fn device_tool_call_line(tool_call: &acp_thread::ToolCall, cx: &App) -> String {
    let label = tool_call.label.read(cx).source().to_string();
    let state = match &tool_call.status {
        acp_thread::ToolCallStatus::Pending => "queued",
        acp_thread::ToolCallStatus::WaitingForConfirmation { .. } => "waiting for approval",
        acp_thread::ToolCallStatus::InProgress => "running",
        acp_thread::ToolCallStatus::Completed => "done",
        acp_thread::ToolCallStatus::Failed => "failed",
        acp_thread::ToolCallStatus::Canceled => "canceled",
        acp_thread::ToolCallStatus::Rejected => "rejected",
    };
    format!("{} — {state}", device_label_preview(&label).trim())
}

/// Remove the reasoning blocks that Zed's Markdown export wraps in
/// `<thinking>` tags.
///
/// The desktop draws a thought as its own collapsed affordance and never as
/// prose, so its Markdown tag has no reader on the phone. An empty thought
/// still exports as a tag pair, which arrived as a literal `<thinking>` and
/// `</thinking>` under the answer.
fn without_thoughts(text: &str) -> String {
    const OPEN: &str = "<thinking>";
    const CLOSE: &str = "</thinking>";
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        kept.push_str(&rest[..start]);
        let after_open = &rest[start + OPEN.len()..];
        match after_open.find(CLOSE) {
            Some(end) => rest = &after_open[end + CLOSE.len()..],
            // A thought that is still streaming has no closing tag yet. Drop
            // the remainder rather than leaking a half-written thought.
            None => {
                rest = "";
                break;
            }
        }
    }
    kept.push_str(rest);
    kept.trim().to_string()
}

fn device_transcript(
    thread_ref: &str,
    entries: &[AgentThreadEntry],
    projected_at: u64,
    cx: &App,
) -> Vec<omega_effectd::MirrorMessage> {
    let start = entries.len().saturating_sub(MAX_DEVICE_TRANSCRIPT_MESSAGES);
    let mut transcript = entries
        .iter()
        .enumerate()
        .skip(start)
        .filter_map(|(index, entry)| {
            let (role, text) = match entry {
                AgentThreadEntry::UserMessage(_) => {
                    (omega_effectd::MessageRole::User, entry.to_markdown(cx))
                }
                AgentThreadEntry::AssistantMessage(message) => (
                    omega_effectd::MessageRole::Assistant,
                    message.to_markdown(cx),
                ),
                // A tool call is most of what a coding turn actually does, and
                // a delegation to another harness is one. Dropping them left
                // the phone showing a question and then a silence while the
                // desktop worked. Mirror the label and the state only: never
                // the arguments, the diff, or the command output.
                AgentThreadEntry::ToolCall(tool_call) => (
                    omega_effectd::MessageRole::Tool,
                    device_tool_call_line(tool_call, cx),
                ),
                AgentThreadEntry::SystemNote(_)
                | AgentThreadEntry::Elicitation(_)
                | AgentThreadEntry::CompletedPlan(_)
                | AgentThreadEntry::ContextCompaction(_) => return None,
            };
            // Reduce the entry to what the phone is allowed to read BEFORE the
            // safety check, not after. The check ran on the raw export, which
            // still carried the model's reasoning, and reasoning names paths
            // and identifiers far more often than an answer does. One
            // disallowed line inside a thought dropped the whole message, so a
            // streaming answer stayed invisible until its reasoning ended and
            // then landed in one piece. The mirror never wanted the thought.
            let text = without_thoughts(without_role_heading(&text));
            if !full_auto_ui::issue31_device_mirror_text_is_safe(&text) {
                return None;
            }
            let preview = device_transcript_preview(&text);
            let text = full_auto_ui::issue31_device_mirror_text(
                &preview,
                MAX_DEVICE_TRANSCRIPT_TEXT_BYTES,
            )?;
            Some(omega_effectd::MirrorMessage {
                message_ref: format!("{thread_ref}.entry.{index}"),
                role,
                text,
                created_at: projected_at,
            })
        })
        .collect::<Vec<_>>();
    while transcript
        .iter()
        .map(|message| message.text.len())
        .sum::<usize>()
        > MAX_DEVICE_TRANSCRIPT_TOTAL_BYTES
    {
        transcript.remove(0);
    }
    transcript
}

fn device_executor_disclosure(
    disclosure: &omega_front_door::ExecutorDisclosure,
) -> omega_effectd::ExecutorDisclosure {
    let executor_name = match disclosure.agent_id.as_str() {
        CODEX_AGENT_ID => "Codex",
        CLAUDE_AGENT_ID => "Claude Code",
        "omega-agent" => "Omega",
        _ => disclosure.agent_id.as_str(),
    };
    omega_effectd::ExecutorDisclosure {
        executor_id: device_public_label(&disclosure.agent_id, "external-acp"),
        executor_name: device_public_label(executor_name, "External executor"),
        model_id: disclosure.model.as_deref().and_then(|model| {
            full_auto_ui::issue31_device_mirror_text(model, MAX_DEVICE_LABEL_BYTES)
        }),
        model_name: disclosure.model.as_deref().and_then(|model| {
            full_auto_ui::issue31_device_mirror_text(model, MAX_DEVICE_LABEL_BYTES)
        }),
    }
}

fn fallback_device_executor_disclosure(thread: &HostThread) -> omega_effectd::ExecutorDisclosure {
    let (executor_id, executor_name) = match thread.lane.as_str() {
        CLAUDE_LOCAL_LANE => (CLAUDE_AGENT_ID, "Claude Code"),
        _ => (CODEX_AGENT_ID, "Codex"),
    };
    omega_effectd::ExecutorDisclosure {
        executor_id: executor_id.to_string(),
        executor_name: executor_name.to_string(),
        model_id: thread
            .turns
            .last()
            .and_then(|turn| turn.model.as_deref())
            .and_then(|model| {
                full_auto_ui::issue31_device_mirror_text(model, MAX_DEVICE_LABEL_BYTES)
            }),
        model_name: thread
            .turns
            .last()
            .and_then(|turn| turn.model.as_deref())
            .and_then(|model| {
                full_auto_ui::issue31_device_mirror_text(model, MAX_DEVICE_LABEL_BYTES)
            }),
    }
}

fn device_thread_state(status: ThreadStatus, thread: &HostThread) -> omega_effectd::ThreadState {
    if status == ThreadStatus::Generating {
        return omega_effectd::ThreadState::Running;
    }
    match thread
        .turns
        .last()
        .and_then(|turn| turn.disposition.as_deref())
    {
        Some("completed") => omega_effectd::ThreadState::Completed,
        Some("failed" | "timed_out" | "owner_interrupted") => omega_effectd::ThreadState::Failed,
        Some(_) => omega_effectd::ThreadState::Waiting,
        None if thread.turns.is_empty() => omega_effectd::ThreadState::Idle,
        None => omega_effectd::ThreadState::Running,
    }
}

fn device_mirror_run(thread: &HostThread, projected_at: u64) -> omega_effectd::MirrorRun {
    let state = match thread
        .turns
        .last()
        .and_then(|turn| turn.disposition.as_deref())
    {
        Some("completed") => omega_effectd::RunState::Completed,
        Some("owner_interrupted") => omega_effectd::RunState::Cancelled,
        Some("failed" | "timed_out") => omega_effectd::RunState::Failed,
        Some(_) => omega_effectd::RunState::Paused,
        None if thread.turns.is_empty() => omega_effectd::RunState::Queued,
        None => omega_effectd::RunState::Running,
    };
    omega_effectd::MirrorRun {
        run_ref: thread.operation_ref.clone(),
        title: thread.operation_ref.clone(),
        lane: thread.lane.clone(),
        state,
        receipt_refs: Vec::new(),
        updated_at: thread
            .turns
            .last()
            .map_or(projected_at, |turn| timestamp_millis(&turn.updated_at)),
    }
}

fn device_transcript_preview(text: &str) -> String {
    if text.len() <= MAX_DEVICE_TRANSCRIPT_TEXT_BYTES {
        return text.to_string();
    }
    let suffix = "\n\n[Transcript preview truncated; full content remains on the desktop.]";
    let budget = MAX_DEVICE_TRANSCRIPT_TEXT_BYTES.saturating_sub(suffix.len());
    format!("{}{suffix}", truncate_utf8(text, budget))
}

fn device_label_preview(text: &str) -> String {
    truncate_utf8(text, MAX_DEVICE_LABEL_BYTES)
}

fn device_public_label(text: &str, fallback: &str) -> String {
    let preview = device_label_preview(text);
    full_auto_ui::issue31_device_mirror_text(&preview, MAX_DEVICE_LABEL_BYTES)
        .unwrap_or_else(|| fallback.to_string())
}

fn timestamp_millis(timestamp: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .and_then(|timestamp| u64::try_from(timestamp.timestamp_millis()).ok())
        .unwrap_or_else(current_unix_millis)
}

fn current_unix_millis() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0)
}

fn publish_device_thread(
    state: &Rc<RefCell<HostBridgeState>>,
    thread_ref: &str,
    cx: &App,
) -> anyhow::Result<()> {
    let (journal, thread) = {
        let state = state.borrow();
        let Some(journal) = state.device_projection.clone() else {
            return Ok(());
        };
        let thread = state
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == thread_ref)
            .cloned();
        (journal, thread)
    };
    let Some(thread) = thread else {
        journal.publish(omega_effectd::MirrorChange::ThreadRemove {
            thread_ref: thread_ref.to_string(),
        })?;
        return Ok(());
    };
    let projected_at = current_unix_millis();
    let projected = device_mirror_thread(&thread, projected_at, cx);
    let previous = journal
        .snapshot()?
        .threads
        .into_iter()
        .find(|candidate| candidate.thread_ref == projected.thread_ref);
    match previous {
        None => {
            journal.publish(omega_effectd::MirrorChange::ThreadUpsert { thread: projected })?;
        }
        Some(previous) if device_thread_metadata_changed(&previous, &projected) => {
            journal.publish(omega_effectd::MirrorChange::ThreadUpsert { thread: projected })?;
        }
        Some(previous) => {
            publish_device_transcript_delta(&journal, &previous, &projected)?;
        }
    }

    if let Some(reading) = full_auto_ui::issue31_device_mirror_reading() {
        let snapshot = journal.snapshot()?;
        for run in reading.runs {
            if !snapshot.runs.iter().any(|existing| existing == &run) {
                journal.publish(omega_effectd::MirrorChange::RunUpsert { run })?;
            }
        }
        let health = omega_effectd::MirrorHealth {
            engine_up: true,
            engine_generation: reading.engine_generation,
            lane_ready: reading.lane_ready,
            observed_at: reading.observed_at_ms,
        };
        if snapshot.health != health {
            journal.publish(omega_effectd::MirrorChange::Health { health })?;
        }
    } else {
        let run = device_mirror_run(&thread, projected_at);
        if !journal
            .snapshot()?
            .runs
            .iter()
            .any(|existing| existing == &run)
        {
            journal.publish(omega_effectd::MirrorChange::RunUpsert { run })?;
        }
    }
    Ok(())
}

fn device_thread_metadata_changed(
    previous: &omega_effectd::MirrorThread,
    current: &omega_effectd::MirrorThread,
) -> bool {
    previous.title != current.title
        || previous.executor != current.executor
        || previous.state != current.state
}

fn publish_device_transcript_delta(
    journal: &omega_effectd::ProjectionJournal,
    previous: &omega_effectd::MirrorThread,
    current: &omega_effectd::MirrorThread,
) -> anyhow::Result<()> {
    if current.transcript.starts_with(&previous.transcript) {
        for message in current.transcript.iter().skip(previous.transcript.len()) {
            journal.publish(omega_effectd::MirrorChange::TranscriptAppend {
                thread_ref: current.thread_ref.clone(),
                message: message.clone(),
                updated_at: current.updated_at,
            })?;
        }
        return Ok(());
    }
    if previous.transcript.len() == current.transcript.len()
        && let (Some(previous_message), Some(current_message)) =
            (previous.transcript.last(), current.transcript.last())
        && previous_message.message_ref == current_message.message_ref
        && previous_message.role == current_message.role
        && current_message.text.starts_with(&previous_message.text)
        && current_message.text.len() > previous_message.text.len()
    {
        let suffix = current_message
            .text
            .get(previous_message.text.len()..)
            .ok_or_else(|| anyhow::anyhow!("device transcript delta split a UTF-8 boundary"))?;
        journal.publish(omega_effectd::MirrorChange::TranscriptAppend {
            thread_ref: current.thread_ref.clone(),
            message: omega_effectd::MirrorMessage {
                message_ref: format!(
                    "{}.segment.{}",
                    current_message.message_ref,
                    previous_message.text.len()
                ),
                role: current_message.role.clone(),
                text: suffix.to_string(),
                created_at: current_message.created_at,
            },
            updated_at: current.updated_at,
        })?;
        return Ok(());
    }
    journal.publish(omega_effectd::MirrorChange::ThreadUpsert {
        thread: current.clone(),
    })?;
    Ok(())
}

fn refresh_device_generation(
    state: &Rc<RefCell<HostBridgeState>>,
    generation: u64,
    cx: &App,
) -> anyhow::Result<()> {
    let journal = state.borrow().device_projection.clone();
    let Some(journal) = journal else {
        return Ok(());
    };
    if journal.cursor()?.generation != generation {
        journal.replace_snapshot(device_mirror_snapshot(&state.borrow(), generation, cx)?)?;
    }
    Ok(())
}

fn sarah_host_error(error: omega_effectd::SarahConversationError) -> HostResponseError {
    let code = host_response_error_code(error.protocol_code());
    HostResponseError {
        code,
        message: error.to_string(),
    }
}

fn host_response_error_code(code: omega_effectd::ProtocolErrorCode) -> HostResponseErrorCode {
    match code {
        omega_effectd::ProtocolErrorCode::StaleGeneration
        | omega_effectd::ProtocolErrorCode::StaleCursor => HostResponseErrorCode::StaleGeneration,
        omega_effectd::ProtocolErrorCode::InvalidRequest
        | omega_effectd::ProtocolErrorCode::FrameTooLarge => HostResponseErrorCode::InvalidRequest,
        omega_effectd::ProtocolErrorCode::UnknownMethod
        | omega_effectd::ProtocolErrorCode::IncompatibleVersion => {
            HostResponseErrorCode::Unsupported
        }
        omega_effectd::ProtocolErrorCode::NotRunning
        | omega_effectd::ProtocolErrorCode::RunNotFound
        | omega_effectd::ProtocolErrorCode::HostUnavailable
        | omega_effectd::ProtocolErrorCode::HostTimeout
        | omega_effectd::ProtocolErrorCode::NotFound
        | omega_effectd::ProtocolErrorCode::Unavailable
        | omega_effectd::ProtocolErrorCode::Gap => HostResponseErrorCode::Unavailable,
        omega_effectd::ProtocolErrorCode::Forbidden => HostResponseErrorCode::Forbidden,
        omega_effectd::ProtocolErrorCode::Conflict => HostResponseErrorCode::Conflict,
        omega_effectd::ProtocolErrorCode::Internal => HostResponseErrorCode::Internal,
    }
}

async fn handle_request(
    request: HostRequestFrame,
    state: Rc<RefCell<HostBridgeState>>,
    openagents_session: OpenAgentsSession,
    mut cx: AsyncApp,
) -> Result<Value, HostResponseError> {
    match request.method.clone() {
        HostMethod::ResolveWorkspace => resolve_workspace(request.params, &state, &mut cx),
        HostMethod::ResolveSyncSession => {
            resolve_sync_session(request.params, &openagents_session, &mut cx).await
        }
        HostMethod::CreateThread => create_thread(request.params, &state, &mut cx),
        HostMethod::LaneReadiness => lane_readiness(request.params, &state, &cx),
        HostMethod::DispatchTurn => dispatch_turn(request.params, &state, &mut cx).await,
        HostMethod::RefreshEvidence => {
            refresh_evidence(request.params, request.generation, &state, &cx)
        }
        HostMethod::InterruptTurn => interrupt_turn(request.params, &state, &mut cx).await,
        HostMethod::AppendSystemNote => append_system_note(request.params, &state, &mut cx).await,
        HostMethod::SarahSessionStatus
        | HostMethod::SarahBootstrap
        | HostMethod::SarahRoomSnapshot
        | HostMethod::SarahSendMessage
        | HostMethod::SarahInterruptTurn
        | HostMethod::SarahDeviceGrants => sarah_request(request, &state, &mut cx).await,
        HostMethod::SarahRenewDeviceGrant | HostMethod::SarahRevokeDeviceGrant => {
            sarah_request(request, &state, &mut cx).await
        }
        HostMethod::Unsupported => Err(unsupported("Unknown Omega host method.")),
    }
}

async fn resolve_sync_session(
    params: Value,
    session: &OpenAgentsSession,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let _: ResolveSyncSessionParams = decode_params(params)?;
    Ok(sync_session_result(session.resolve_verified(cx).await))
}

fn sync_session_result(session: Option<VerifiedOpenAgentsSession>) -> Value {
    match session {
        Some(session) => json!({
            "available": true,
            "baseUrl": session.base_url,
            "accessToken": session.access_token,
        }),
        None => json!({ "available": false }),
    }
}

fn resolve_workspace(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    require_loaded_journal(state)?;
    let params: ResolveWorkspaceParams = decode_params(params)?;
    if let Some(expected) = params.expected_workspace_ref.as_deref() {
        validate_ref(expected, "expectedWorkspaceRef")?;
    }
    if let Some(binding) = state.borrow().workspace.clone()
        && binding.workspace.upgrade().is_some()
    {
        if params
            .expected_workspace_ref
            .as_ref()
            .is_some_and(|expected| expected != &binding.workspace_ref)
        {
            return Err(unavailable("The requested workspace is not active."));
        }
        return Ok(json!({ "workspaceRef": binding.workspace_ref }));
    }

    let candidates = cx.update(|cx| {
        let app_state = AppState::global(cx);
        app_state
            .workspace_store
            .read(cx)
            .workspaces_with_windows()
            .filter_map(|(window, workspace)| {
                let workspace = workspace.upgrade()?;
                let has_worktree = workspace
                    .read(cx)
                    .project()
                    .read(cx)
                    .worktrees(cx)
                    .next()
                    .is_some();
                has_worktree.then(|| (window, workspace))
            })
            .collect::<Vec<_>>()
    });
    let [(window, workspace)] = candidates.as_slice() else {
        return Err(unavailable(
            "Omega requires exactly one open workspace with a worktree.",
        ));
    };
    let workspace_ref = params
        .expected_workspace_ref
        .unwrap_or_else(|| SUPERVISED_WORKSPACE_REF.to_string());
    state.borrow_mut().workspace = Some(WorkspaceBinding {
        workspace_ref: workspace_ref.clone(),
        workspace: workspace.downgrade(),
        window: *window,
    });
    rebind_persisted_threads(&workspace_ref, state, cx)?;
    Ok(json!({ "workspaceRef": workspace_ref }))
}

fn create_thread(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: CreateThreadParams = decode_params(params)?;
    validate_ref(&params.operation_ref, "operationRef")?;
    validate_ref(&params.workspace_ref, "workspaceRef")?;
    if params.title.trim().is_empty() {
        return Err(invalid("title must not be empty."));
    }
    let agent = agent_for_lane(&params.lane)?;
    if let Some(thread_ref) = state.borrow().threads.iter().find_map(|thread| {
        (thread.workspace_ref == params.workspace_ref
            && thread.lane == params.lane
            && thread.operation_ref == params.operation_ref)
            .then(|| thread.thread_id.to_key_string())
    }) {
        return Ok(json!({ "threadRef": thread_ref }));
    }
    let binding = require_workspace(&params.workspace_ref, state)?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| unavailable("The bound workspace was closed."))?;
    let (thread_id, conversation) = binding
        .window
        .update(cx, |_root, window, cx| {
            let panel = workspace
                .read(cx)
                .panel::<AgentPanel>(cx)
                .ok_or_else(|| unavailable("The workspace Agent panel is unavailable."))?;
            let thread_id = panel.update(cx, |panel, cx| {
                panel.create_thread_with_options(
                    CreateThreadOptions {
                        title: Some(params.title.clone().into()),
                        agent: Some(agent),
                        ..Default::default()
                    },
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            });
            let conversation = panel
                .read(cx)
                .conversation_view_for_id(&thread_id, cx)
                .cloned()
                .ok_or_else(|| internal("The created thread was not retained."))?;
            Ok((thread_id, conversation))
        })
        .map_err(|error| unavailable(format!("The workspace window is unavailable: {error}")))??;
    let thread_ref = thread_id.to_key_string();
    state.borrow_mut().threads.push(HostThread {
        workspace_ref: params.workspace_ref,
        lane: params.lane,
        operation_ref: params.operation_ref,
        thread_id,
        conversation: Some(conversation.downgrade()),
        turns: Vec::new(),
        revision: 1,
    });
    persist_state(state)?;
    cx.update(|cx| {
        publish_device_thread(state, &thread_ref, cx)
            .map_err(|error| internal(format!("Device thread projection failed: {error:#}")))
    })?;
    Ok(json!({ "threadRef": thread_ref }))
}

fn lane_readiness(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: LaneReadinessParams = decode_params(params)?;
    validate_lane(&params.lane)?;
    if let Some(thread_ref) = params.excluding_thread_ref.as_deref() {
        validate_ref(thread_ref, "excludingThreadRef")?;
    }
    if !is_supported_lane(&params.lane) {
        return Ok(json!({
            "known": false,
            "admitted": false,
            "fullAuto": false,
            "state": "unavailable",
        }));
    }
    let workspace_ready = state
        .borrow()
        .workspace
        .as_ref()
        .is_some_and(|binding| binding.workspace.upgrade().is_some());
    let dispatch_thread_exists =
        params
            .excluding_thread_ref
            .as_ref()
            .is_some_and(|excluding_thread_ref| {
                state.borrow().threads.iter().any(|thread| {
                    thread.lane == params.lane
                        && thread.thread_id.to_key_string() == *excluding_thread_ref
                })
            });
    let (authority_ready, authority_state) = match params.lane.as_str() {
        CODEX_LOCAL_LANE => {
            external_agent_lane_readiness(state, cx, CODEX_AGENT_ID, dispatch_thread_exists)?
        }
        CLAUDE_LOCAL_LANE => {
            external_agent_lane_readiness(state, cx, CLAUDE_AGENT_ID, dispatch_thread_exists)?
        }
        _ => return Err(unsupported("The provider lane is not supported.")),
    };
    let busy = state.borrow().threads.iter().any(|host_thread| {
        if host_thread.lane != params.lane {
            return false;
        }
        if params
            .excluding_thread_ref
            .as_ref()
            .is_some_and(|thread_ref| thread_ref == &host_thread.thread_id.to_key_string())
        {
            return false;
        }
        let Some(conversation) = host_thread
            .conversation
            .as_ref()
            .and_then(WeakEntity::upgrade)
        else {
            return false;
        };
        cx.update(|cx| {
            conversation
                .read(cx)
                .root_thread(cx)
                .is_some_and(|thread| thread.read(cx).status() == ThreadStatus::Generating)
        })
    });
    let admitted = workspace_ready && authority_ready;
    Ok(json!({
        "known": true,
        "admitted": admitted,
        "fullAuto": admitted,
        "state": if admitted && !busy { "available" } else if busy { "busy" } else { authority_state },
    }))
}

async fn dispatch_turn(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: DispatchTurnParams = decode_params(params)?;
    validate_ref(&params.run_ref, "runRef")?;
    validate_ref(&params.workspace_ref, "workspaceRef")?;
    validate_ref(&params.thread_ref, "threadRef")?;
    validate_ref(&params.turn_ref, "turnRef")?;
    if params.message.is_empty() {
        return Err(invalid("message must not be empty."));
    }
    require_workspace(&params.workspace_ref, state)?;
    let lane = params
        .profile
        .as_ref()
        .and_then(|profile| profile.lane.clone())
        .unwrap_or_else(|| CODEX_LOCAL_LANE.to_string());
    if !is_supported_lane(&lane) {
        return Ok(json!({
            "accepted": false,
            "reason": "unsupported_lane",
            "failureCause": "lane_unavailable",
        }));
    }
    validate_lane(&lane)?;
    if let Some(profile) = params.profile.as_ref() {
        if let Some(account_ref) = profile.account_ref.as_deref() {
            validate_ref(account_ref, "profile.accountRef")?;
        }
        if let Some(model) = profile.model.as_deref() {
            validate_ref(model, "profile.model")?;
        }
        if let Some(reasoning_effort) = profile.reasoning_effort.as_deref() {
            validate_ref(reasoning_effort, "profile.reasoningEffort")?;
        }
    }
    if params
        .profile
        .as_ref()
        .is_some_and(|profile| profile.model.is_some() || profile.reasoning_effort.is_some())
    {
        return Ok(json!({
            "accepted": false,
            "reason": "profile_override_unavailable",
            "failureCause": "lane_unavailable",
        }));
    }
    let conversation = {
        let bridge = state.borrow();
        let host_thread = bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?;
        if host_thread.workspace_ref != params.workspace_ref {
            return Err(unavailable("The thread belongs to a different workspace."));
        }
        if host_thread.lane != lane {
            return Err(invalid(
                "The dispatch lane does not match the Agent thread lane.",
            ));
        }
        host_thread
            .conversation
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .ok_or_else(|| unavailable("The requested Agent thread was closed."))?
    };
    let thread = wait_for_root_thread(&conversation, cx).await?;
    let (start_entry_index, provider_session_ref) = cx.update(|cx| {
        let thread = thread.read(cx);
        if thread.status() == ThreadStatus::Generating {
            return Err(unavailable("The Agent thread already has a running turn."));
        }
        Ok((thread.entries().len(), thread.session_id().0.to_string()))
    })?;
    validate_ref(&provider_session_ref, "providerSessionRef")?;
    let turn_timeout = turn_timeout_for_lane(&lane);
    let now = Utc::now().to_rfc3339();
    {
        let mut bridge = state.borrow_mut();
        let host_thread = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?;
        if host_thread
            .turns
            .iter()
            .any(|turn| turn.turn_ref == params.turn_ref)
        {
            return Ok(json!({ "accepted": false, "reason": "duplicate_turn_ref" }));
        }
        host_thread.turns.push(HostTurn {
            turn_ref: params.turn_ref.clone(),
            lane,
            account_ref: params
                .profile
                .as_ref()
                .and_then(|profile| profile.account_ref.clone()),
            model: params
                .profile
                .as_ref()
                .and_then(|profile| profile.model.clone()),
            provider_session_ref,
            start_entry_index,
            end_entry_index: None,
            phase: "streaming".to_string(),
            disposition: None,
            created_at: now.clone(),
            updated_at: now,
        });
        host_thread.revision += 1;
    }
    persist_state(state)?;
    let send = thread.update(cx, |thread, cx| {
        thread.send(vec![params.message.into()], cx)
    });
    cx.update(|cx| {
        publish_device_thread(state, &params.thread_ref, cx)
            .map_err(|error| internal(format!("Device turn projection failed: {error:#}")))
    })?;
    let state_for_stream = state.clone();
    let thread_for_stream = thread.clone();
    let thread_ref_for_stream = params.thread_ref.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(DEVICE_TRANSCRIPT_PUBLISH_INTERVAL)
                .await;
            let (generating, projection_result) = cx.update(|cx| {
                let generating = thread_for_stream.read(cx).status() == ThreadStatus::Generating;
                let projection_result =
                    publish_device_thread(&state_for_stream, &thread_ref_for_stream, cx);
                (generating, projection_result)
            });
            if let Err(error) = projection_result {
                log::error!("failed to publish streaming device thread projection: {error:#}");
            }
            if !generating {
                break;
            }
        }
    })
    .detach();
    let turn_ref = params.turn_ref;
    let thread_ref_for_completion = params.thread_ref;
    let state_for_completion = state.clone();
    let thread_for_completion = thread.clone();
    cx.spawn(async move |cx| {
        let outcome = if let Some(timeout) = turn_timeout {
            let timer = cx.background_executor().timer(timeout);
            match futures::future::select(send, timer).await {
                futures::future::Either::Left((result, _)) => {
                    if result.is_ok() {
                        HostTurnOutcome::Completed
                    } else {
                        HostTurnOutcome::Failed
                    }
                }
                futures::future::Either::Right(((), _)) => {
                    log::warn!("Omega Full Auto Claude turn exceeded its host deadline");
                    let cancel = cx.update(|cx| {
                        thread_for_completion.update(cx, |thread, cx| {
                            (thread.status() == ThreadStatus::Generating).then(|| thread.cancel(cx))
                        })
                    });
                    if let Some(cancel) = cancel {
                        let cancel_deadline =
                            cx.background_executor().timer(ACP_CANCEL_GRACE_PERIOD);
                        if matches!(
                            futures::future::select(cancel, cancel_deadline).await,
                            futures::future::Either::Right(_)
                        ) {
                            log::warn!(
                                "Omega Full Auto Claude cancellation exceeded its grace period"
                            );
                        }
                    }
                    HostTurnOutcome::TimedOut
                }
            }
        } else if send.await.is_ok() {
            HostTurnOutcome::Completed
        } else {
            HostTurnOutcome::Failed
        };
        let (end_entry_index, had_error) = cx.update(|cx| {
            let thread = thread_for_completion.read(cx);
            (thread.entries().len(), thread.had_error())
        });
        let outcome = if outcome == HostTurnOutcome::Completed && had_error {
            HostTurnOutcome::Failed
        } else {
            outcome
        };
        if let Err(error) =
            record_turn_completion(&state_for_completion, &turn_ref, end_entry_index, outcome)
        {
            log::error!("failed to persist Omega Full Auto host correlation: {error:#}");
        }
        if let Err(error) = cx.update(|cx| {
            publish_device_thread(&state_for_completion, &thread_ref_for_completion, cx)
        }) {
            log::error!("failed to publish completed device thread projection: {error:#}");
        }
    })
    .detach();
    Ok(json!({ "accepted": true }))
}

fn refresh_evidence(
    params: Value,
    generation: u64,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: RefreshEvidenceParams = decode_params(params)?;
    validate_ref(&params.run_ref, "runRef")?;
    validate_ref(&params.thread_ref, "threadRef")?;
    let (conversation, mut turns, mut revision) = {
        let bridge = state.borrow();
        let Some(thread) = bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
        else {
            return Ok(json!({ "present": false, "revision": 0, "live": null, "turns": [] }));
        };
        (
            thread.conversation.as_ref().and_then(WeakEntity::upgrade),
            thread.turns.clone(),
            thread.revision,
        )
    };
    if turns.len() > MAX_EVIDENCE_TURNS {
        turns.drain(..turns.len() - MAX_EVIDENCE_TURNS);
    }
    let Some(conversation) = conversation else {
        return Ok(json!({ "present": false, "revision": revision, "live": null, "turns": [] }));
    };
    let Some(thread) = cx.update(|cx| conversation.read(cx).root_thread(cx)) else {
        return Ok(json!({ "present": true, "revision": revision, "live": null, "turns": [] }));
    };
    let (status, entry_count, assistant_texts) = cx.update(|cx| {
        let thread = thread.read(cx);
        let mut texts = turns
            .iter()
            .map(|turn| {
                let end_entry_index = turn.end_entry_index.unwrap_or(thread.entries().len());
                let text = thread
                    .entries()
                    .iter()
                    .skip(turn.start_entry_index)
                    .take(end_entry_index.saturating_sub(turn.start_entry_index))
                    .filter_map(|entry| match entry {
                        AgentThreadEntry::AssistantMessage(message) => {
                            Some(message.to_markdown(cx))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                truncate_utf8(&text, MAX_ASSISTANT_TEXT_BYTES)
            })
            .collect::<Vec<_>>();
        let mut remaining_bytes = MAX_TOTAL_ASSISTANT_TEXT_BYTES;
        for text in texts.iter_mut().rev() {
            *text = truncate_utf8(text, text.len().min(remaining_bytes));
            remaining_bytes = remaining_bytes.saturating_sub(text.len());
        }
        (thread.status(), thread.entries().len(), texts)
    });
    if status == ThreadStatus::Idle
        && let Some(turn) = turns
            .iter_mut()
            .rev()
            .find(|turn| turn.disposition.is_none())
    {
        turn.phase = "completed".to_string();
        turn.disposition = Some("completed".to_string());
        turn.end_entry_index = Some(entry_count);
        turn.updated_at = Utc::now().to_rfc3339();
        {
            let mut bridge = state.borrow_mut();
            if let Some(host_thread) = bridge
                .threads
                .iter_mut()
                .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
                && let Some(stored_turn) = host_thread
                    .turns
                    .iter_mut()
                    .find(|stored_turn| stored_turn.turn_ref == turn.turn_ref)
                && stored_turn.disposition.is_none()
            {
                *stored_turn = turn.clone();
                host_thread.revision += 1;
                revision = host_thread.revision;
            }
        }
        persist_state(state)?;
    }
    let active_turn = turns.iter().rev().find(|turn| turn.disposition.is_none());
    let last_turn = turns.last();
    let live = if let Some(turn) = active_turn {
        json!({ "state": "turn_running", "turnRef": turn.turn_ref })
    } else if let Some(turn) = last_turn {
        match turn.disposition.as_deref() {
            Some("completed") => json!({ "state": "turn_completed", "turnRef": turn.turn_ref }),
            Some("failed") => json!({
                "state": "blocked",
                "turnRef": turn.turn_ref,
                "reason": "agent_turn_failed",
            }),
            Some("timed_out") => json!({
                "state": "blocked",
                "turnRef": turn.turn_ref,
                "reason": "provider_turn_timed_out",
            }),
            Some("owner_interrupted") => json!({
                "state": "blocked",
                "turnRef": turn.turn_ref,
                "reason": "owner_interrupted",
            }),
            _ => Value::Null,
        }
    } else {
        Value::Null
    };
    let turn_records = turns
        .iter()
        .zip(assistant_texts)
        .map(|(turn, assistant_text)| {
            let user_message_key = message_key(&turn.turn_ref, "user");
            let assistant_message_key = message_key(&turn.turn_ref, "assistant");
            let assistant_segments = if assistant_text.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "key": assistant_message_key,
                    "text": assistant_text,
                })]
            };
            json!({
                "schema": "openagents.desktop.local_turn_record.v1",
                "threadRef": params.thread_ref,
                "turnRef": turn.turn_ref,
                "lane": turn.lane,
                "userMessageKey": user_message_key,
                "assistantMessageKey": assistant_message_key,
                "accountRef": turn.account_ref,
                "providerSessionRef": turn.provider_session_ref,
                "model": turn.model,
                "phase": turn.phase,
                "persistedCursor": assistant_text.len(),
                "assistantText": assistant_text,
                "assistantSegments": assistant_segments,
                "recoveryGeneration": generation,
                "disposition": turn.disposition,
                "createdAt": turn.created_at,
                "updatedAt": turn.updated_at,
            })
        })
        .collect::<Vec<_>>();
    cx.update(|cx| {
        publish_device_thread(state, &params.thread_ref, cx)
            .map_err(|error| internal(format!("Device evidence projection failed: {error:#}")))
    })?;
    Ok(json!({
        "present": true,
        "revision": revision,
        "live": live,
        "turns": turn_records,
    }))
}

async fn interrupt_turn(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: InterruptTurnParams = decode_params(params)?;
    validate_ref(&params.thread_ref, "threadRef")?;
    if let Some(turn_ref) = params.turn_ref.as_deref() {
        validate_ref(turn_ref, "turnRef")?;
    }
    let (conversation, turn_ref) = {
        let bridge = state.borrow();
        let host_thread = bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?;
        let turn_ref = params.turn_ref.clone().or_else(|| {
            host_thread
                .turns
                .iter()
                .rev()
                .find(|turn| turn.disposition.is_none())
                .map(|turn| turn.turn_ref.clone())
        });
        (
            host_thread
                .conversation
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .ok_or_else(|| unavailable("The requested Agent thread was closed."))?,
            turn_ref,
        )
    };
    let Some(turn_ref) = turn_ref else {
        return Ok(json!({ "interrupted": false }));
    };
    let thread = wait_for_root_thread(&conversation, cx).await?;
    let cancel = thread.update(cx, |thread, cx| {
        if thread.status() != ThreadStatus::Generating {
            None
        } else {
            Some(thread.cancel(cx))
        }
    });
    let Some(cancel) = cancel else {
        return Ok(json!({ "interrupted": false }));
    };
    cancel.await;
    let end_entry_index = cx.update(|cx| thread.read(cx).entries().len());
    {
        let mut bridge = state.borrow_mut();
        if let Some(host_thread) = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            && let Some(turn) = host_thread
                .turns
                .iter_mut()
                .find(|turn| turn.turn_ref == turn_ref)
        {
            turn.phase = "interrupted".to_string();
            turn.disposition = Some("owner_interrupted".to_string());
            turn.end_entry_index = Some(end_entry_index);
            turn.updated_at = Utc::now().to_rfc3339();
            host_thread.revision += 1;
        }
    }
    persist_state(state)?;
    cx.update(|cx| {
        publish_device_thread(state, &params.thread_ref, cx)
            .map_err(|error| internal(format!("Device interruption projection failed: {error:#}")))
    })?;
    Ok(json!({ "interrupted": true }))
}

/// OMEGA-DELTA-0045. Write a host-authored note into the thread the owner
/// reads.
///
/// This used to refuse — `unavailable("Agent threads do not expose an
/// owner-visible system-note authority.")` — because `AgentThreadEntry` had no
/// variant a non-model disclosure could be. The refusal was typed and honest,
/// and it was still the rc11 silence: the engine emits a provider-handoff note
/// naming both lanes, the host dropped it, and a run that changed which model
/// was spending the owner's budget left nothing in the transcript. FA-07 gate
/// 5 exists to forbid exactly that.
///
/// The note goes to the thread named in `threadRef` and nowhere else, so a
/// handoff addressed to the *target* thread cannot be filed against the source
/// one. `noteRef` makes the append idempotent: the engine may retry after a
/// response it never saw, and the owner must not be shown the same disclosure
/// twice — nor a rewritten one, which is why the id wins over the newer text.
///
/// The idempotence is scoped to the live thread, not to the correlation
/// journal. That is the scope that matters: the note is an entry in the
/// thread, so a thread that is gone has no owner reading it and nothing to
/// double.
async fn append_system_note(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: AppendSystemNoteParams = decode_params(params)?;
    validate_ref(&params.thread_ref, "threadRef")?;
    validate_ref(&params.note_ref, "noteRef")?;
    if params.text.is_empty() {
        return Err(invalid("text must not be empty."));
    }
    let conversation = {
        let bridge = state.borrow();
        bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?
            .conversation
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .ok_or_else(|| unavailable("The requested Agent thread was closed."))?
    };
    let thread = wait_for_root_thread(&conversation, cx).await?;
    let appended = thread.update(cx, |thread, cx| {
        thread.push_system_note(
            acp_thread::SystemNote {
                id: acp_thread::SystemNoteId(params.note_ref.as_str().into()),
                text: params.text.into(),
            },
            cx,
        )
    });
    cx.update(|cx| {
        publish_device_thread(state, &params.thread_ref, cx)
            .map_err(|error| internal(format!("Device note projection failed: {error:#}")))
    })?;
    Ok(json!({ "appended": appended }))
}

async fn wait_for_root_thread(
    conversation: &Entity<ConversationView>,
    cx: &mut AsyncApp,
) -> Result<Entity<acp_thread::AcpThread>, HostResponseError> {
    for _ in 0..THREAD_CONNECT_ATTEMPTS {
        if let Some(thread) = cx.update(|cx| conversation.read(cx).root_thread(cx)) {
            return Ok(thread);
        }
        cx.background_executor()
            .timer(THREAD_CONNECT_INTERVAL)
            .await;
    }
    Err(unavailable(
        "The Agent thread did not connect before the host deadline.",
    ))
}

fn require_workspace(
    workspace_ref: &str,
    state: &Rc<RefCell<HostBridgeState>>,
) -> Result<WorkspaceBinding, HostResponseError> {
    let binding = state
        .borrow()
        .workspace
        .clone()
        .ok_or_else(|| unavailable("No Omega workspace is bound."))?;
    if binding.workspace_ref != workspace_ref {
        return Err(unavailable("The requested workspace is not bound."));
    }
    if binding.workspace.upgrade().is_none() {
        return Err(unavailable("The bound workspace was closed."));
    }
    Ok(binding)
}

fn require_loaded_journal(state: &Rc<RefCell<HostBridgeState>>) -> Result<(), HostResponseError> {
    if let Some(error) = state.borrow().load_error.as_deref() {
        Err(internal(format!(
            "Omega Full Auto host correlation could not be loaded: {error}"
        )))
    } else {
        Ok(())
    }
}

fn rebind_persisted_threads(
    workspace_ref: &str,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<(), HostResponseError> {
    let thread_ids = state
        .borrow()
        .threads
        .iter()
        .filter(|thread| {
            thread.workspace_ref == workspace_ref
                && thread
                    .conversation
                    .as_ref()
                    .and_then(WeakEntity::upgrade)
                    .is_none()
        })
        .map(|thread| (thread.thread_id, thread.lane.clone()))
        .collect::<Vec<_>>();
    if thread_ids.is_empty() {
        return Ok(());
    }

    let binding = require_workspace(workspace_ref, state)?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| unavailable("The bound workspace was closed."))?;
    let rebound = binding
        .window
        .update(cx, |_root, window, cx| {
            let panel = workspace
                .read(cx)
                .panel::<AgentPanel>(cx)
                .ok_or_else(|| unavailable("The workspace Agent panel is unavailable."))?;
            let mut rebound = Vec::with_capacity(thread_ids.len());
            for (thread_id, lane) in thread_ids {
                let agent = agent_for_lane(&lane)?;
                let conversation = panel.update(cx, |panel, cx| {
                    if panel.conversation_view_for_id(&thread_id, cx).is_none() {
                        panel.load_agent_thread(
                            agent,
                            thread_id,
                            None,
                            None,
                            false,
                            AgentThreadSource::AgentPanel,
                            window,
                            cx,
                        );
                    }
                    panel.conversation_view_for_id(&thread_id, cx).cloned()
                });
                let conversation = conversation.ok_or_else(|| {
                    unavailable("The persisted Full Auto Agent thread could not be restored.")
                })?;
                rebound.push((thread_id, conversation.downgrade()));
            }
            Ok(rebound)
        })
        .map_err(|error| unavailable(format!("The workspace window is unavailable: {error}")))??;

    let mut bridge = state.borrow_mut();
    for (thread_id, conversation) in rebound {
        if let Some(thread) = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id == thread_id)
        {
            thread.conversation = Some(conversation);
        }
    }
    Ok(())
}

fn load_correlation_journal(path: &Path) -> anyhow::Result<Vec<HostThread>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let journal: CorrelationJournal = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        journal.schema == CORRELATION_SCHEMA,
        "unsupported correlation schema {}",
        journal.schema
    );
    journal
        .threads
        .into_iter()
        .map(|thread| {
            Ok(HostThread {
                workspace_ref: thread.workspace_ref,
                lane: thread.lane,
                operation_ref: thread.operation_ref,
                thread_id: ThreadId::from_key_string(&thread.thread_ref)?,
                conversation: None,
                turns: thread.turns,
                revision: thread.revision,
            })
        })
        .collect()
}

fn persist_state(state: &Rc<RefCell<HostBridgeState>>) -> Result<(), HostResponseError> {
    persist_correlation_journal(&state.borrow()).map_err(|error| {
        internal(format!(
            "Omega Full Auto host correlation could not be persisted: {error:#}"
        ))
    })
}

fn record_turn_completion(
    state: &Rc<RefCell<HostBridgeState>>,
    turn_ref: &str,
    end_entry_index: usize,
    outcome: HostTurnOutcome,
) -> anyhow::Result<()> {
    {
        let mut bridge = state.borrow_mut();
        if let Some(host_thread) = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.turns.iter().any(|turn| turn.turn_ref == turn_ref))
        {
            let changed = if let Some(turn) = host_thread
                .turns
                .iter_mut()
                .find(|turn| turn.turn_ref == turn_ref)
            {
                if turn.disposition.is_none() {
                    match outcome {
                        HostTurnOutcome::Completed => {
                            turn.phase = "completed".to_string();
                            turn.disposition = Some("completed".to_string());
                        }
                        HostTurnOutcome::Failed => {
                            turn.phase = "failed".to_string();
                            turn.disposition = Some("failed".to_string());
                        }
                        HostTurnOutcome::TimedOut => {
                            turn.phase = "failed".to_string();
                            turn.disposition = Some("timed_out".to_string());
                        }
                    }
                    turn.end_entry_index = Some(end_entry_index);
                    turn.updated_at = Utc::now().to_rfc3339();
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if changed {
                host_thread.revision += 1;
            }
        }
    }
    persist_correlation_journal(&state.borrow())
}

fn turn_timeout_for_lane(lane: &str) -> Option<Duration> {
    (lane == CLAUDE_LOCAL_LANE).then_some(CLAUDE_TURN_TIMEOUT)
}

fn persist_correlation_journal(state: &HostBridgeState) -> anyhow::Result<()> {
    // OMEGA-DELTA-0021. Every mutation of `state.threads` reaches disk through
    // here, so republishing here is what keeps the lane index and the durable
    // journal from drifting apart within a session.
    republish_engine_lane_runs(&state.threads);
    let journal = CorrelationJournal {
        schema: CORRELATION_SCHEMA.to_string(),
        threads: state
            .threads
            .iter()
            .map(|thread| PersistedHostThread {
                workspace_ref: thread.workspace_ref.clone(),
                lane: thread.lane.clone(),
                operation_ref: thread.operation_ref.clone(),
                thread_ref: thread.thread_id.to_key_string(),
                turns: thread.turns.clone(),
                revision: thread.revision,
            })
            .collect(),
    };
    let parent = state
        .correlation_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("correlation path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temporary_path = state.correlation_path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&journal)?;
    let mut temporary_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)?;
    temporary_file.write_all(&bytes)?;
    temporary_file.sync_all()?;
    std::fs::rename(&temporary_path, &state.correlation_path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn agent_for_lane(lane: &str) -> Result<Agent, HostResponseError> {
    match lane {
        CODEX_LOCAL_LANE => Ok(Agent::Custom {
            id: CODEX_AGENT_ID.into(),
        }),
        CLAUDE_LOCAL_LANE => Ok(Agent::Custom {
            id: CLAUDE_AGENT_ID.into(),
        }),
        _ => Err(unsupported(format!("The {lane} lane is not supported."))),
    }
}

fn is_supported_lane(lane: &str) -> bool {
    matches!(lane, CODEX_LOCAL_LANE | CLAUDE_LOCAL_LANE)
}

fn external_agent_lane_readiness(
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
    agent_id: &'static str,
    dispatch_thread_exists: bool,
) -> Result<(bool, &'static str), HostResponseError> {
    let binding = state
        .borrow()
        .workspace
        .clone()
        .ok_or_else(|| unavailable("No Omega workspace is bound."))?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| unavailable("The bound workspace was closed."))?;
    let agent = Agent::Custom {
        id: agent_id.into(),
    };
    let (registered, connection_status) = cx.update(|cx| {
        let project = workspace.read(cx).project().clone();
        let registered = project
            .read(cx)
            .agent_server_store()
            .read(cx)
            .external_agents()
            .any(|registered_agent_id| registered_agent_id.as_ref() == agent_id);
        let connection_status = workspace
            .read(cx)
            .panel::<AgentPanel>(cx)
            .map(|panel| {
                panel
                    .read(cx)
                    .connection_store()
                    .read(cx)
                    .connection_status(&agent, cx)
            })
            .unwrap_or(AgentConnectionStatus::Disconnected);
        (registered, connection_status)
    });
    Ok(external_agent_authority_state(
        registered,
        connection_status,
        dispatch_thread_exists,
    ))
}

fn external_agent_authority_state(
    registered: bool,
    connection_status: AgentConnectionStatus,
    dispatch_thread_exists: bool,
) -> (bool, &'static str) {
    if !registered {
        return (false, "unavailable");
    }
    match connection_status {
        AgentConnectionStatus::Connected => (true, "available"),
        // Dispatch waits for this exact retained thread's root ACP session, so
        // treating its bootstrap as unavailable would strand the run at zero turns.
        AgentConnectionStatus::Connecting if dispatch_thread_exists => (true, "available"),
        AgentConnectionStatus::Connecting => (false, "connecting"),
        AgentConnectionStatus::Disconnected => (true, "available"),
    }
}

fn decode_params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, HostResponseError> {
    serde_json::from_value(params)
        .map_err(|error| invalid(format!("Invalid host request parameters: {error}")))
}

fn validate_ref(value: &str, name: &str) -> Result<(), HostResponseError> {
    if value.is_empty() || value.len() > 180 {
        Err(invalid(format!("{name} must contain 1 to 180 bytes.")))
    } else {
        Ok(())
    }
}

fn validate_lane(value: &str) -> Result<(), HostResponseError> {
    if value.is_empty() || value.len() > 64 {
        Err(invalid("lane must contain 1 to 64 bytes."))
    } else {
        Ok(())
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

fn message_key(turn_ref: &str, role: &str) -> String {
    let name = format!("{turn_ref}.{role}");
    format!(
        "omega.{role}.{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    )
}

fn invalid(message: impl Into<String>) -> HostResponseError {
    HostResponseError {
        code: HostResponseErrorCode::InvalidRequest,
        message: message.into(),
    }
}

fn unsupported(message: impl Into<String>) -> HostResponseError {
    HostResponseError {
        code: HostResponseErrorCode::Unsupported,
        message: message.into(),
    }
}

fn unavailable(message: impl Into<String>) -> HostResponseError {
    HostResponseError::unavailable(message)
}

fn internal(message: impl Into<String>) -> HostResponseError {
    HostResponseError {
        code: HostResponseErrorCode::Internal,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_front_door::{ExecutorClass, ExecutorDisclosure};
    use tempfile::tempdir;

    /// `ENGINE_LANE_RUNS` is process-wide and every publication replaces it
    /// wholesale, so two tests publishing at once would clobber each other's
    /// assertions. Every test that publishes takes this first.
    static LANE_INDEX_GUARD: Mutex<()> = Mutex::new(());

    fn lane_index_guard() -> std::sync::MutexGuard<'static, ()> {
        LANE_INDEX_GUARD.lock().unwrap_or_else(|poisoned| {
            LANE_INDEX_GUARD.clear_poison();
            poisoned.into_inner()
        })
    }

    #[test]
    fn validates_bounded_refs_and_params() {
        assert!(validate_ref("turn.full-auto.1", "turnRef").is_ok());
        assert!(validate_ref("", "turnRef").is_err());
        assert!(validate_ref(&"x".repeat(181), "turnRef").is_err());
        assert!(decode_params::<ResolveWorkspaceParams>(json!({})).is_ok());
        assert!(decode_params::<ResolveWorkspaceParams>(json!({ "extra": true })).is_err());
        assert!(decode_params::<ResolveSyncSessionParams>(json!({})).is_ok());
        assert!(decode_params::<ResolveSyncSessionParams>(json!({ "token": "no" })).is_err());
    }

    #[test]
    fn maps_every_effectd_protocol_error_to_the_host_contract() {
        use omega_effectd::ProtocolErrorCode;

        let cases = [
            (
                ProtocolErrorCode::StaleGeneration,
                HostResponseErrorCode::StaleGeneration,
            ),
            (
                ProtocolErrorCode::StaleCursor,
                HostResponseErrorCode::StaleGeneration,
            ),
            (
                ProtocolErrorCode::InvalidRequest,
                HostResponseErrorCode::InvalidRequest,
            ),
            (
                ProtocolErrorCode::FrameTooLarge,
                HostResponseErrorCode::InvalidRequest,
            ),
            (
                ProtocolErrorCode::UnknownMethod,
                HostResponseErrorCode::Unsupported,
            ),
            (
                ProtocolErrorCode::IncompatibleVersion,
                HostResponseErrorCode::Unsupported,
            ),
            (
                ProtocolErrorCode::NotRunning,
                HostResponseErrorCode::Unavailable,
            ),
            (
                ProtocolErrorCode::RunNotFound,
                HostResponseErrorCode::Unavailable,
            ),
            (
                ProtocolErrorCode::HostUnavailable,
                HostResponseErrorCode::Unavailable,
            ),
            (
                ProtocolErrorCode::HostTimeout,
                HostResponseErrorCode::Unavailable,
            ),
            (
                ProtocolErrorCode::NotFound,
                HostResponseErrorCode::Unavailable,
            ),
            (
                ProtocolErrorCode::Unavailable,
                HostResponseErrorCode::Unavailable,
            ),
            (ProtocolErrorCode::Gap, HostResponseErrorCode::Unavailable),
            (
                ProtocolErrorCode::Forbidden,
                HostResponseErrorCode::Forbidden,
            ),
            (ProtocolErrorCode::Conflict, HostResponseErrorCode::Conflict),
            (ProtocolErrorCode::Internal, HostResponseErrorCode::Internal),
        ];

        for (protocol, expected) in cases {
            assert_eq!(host_response_error_code(protocol), expected);
        }
    }

    #[test]
    fn sync_session_host_result_is_unavailable_or_runtime_only_verified_material() {
        assert_eq!(sync_session_result(None), json!({ "available": false }));
        assert_eq!(
            sync_session_result(Some(VerifiedOpenAgentsSession {
                base_url: "https://openagents.com".to_string(),
                owner_user_id: "fixture-owner".to_string(),
                access_token: "runtime-only-fixture".to_string(),
            })),
            json!({
                "available": true,
                "baseUrl": "https://openagents.com",
                "accessToken": "runtime-only-fixture",
            })
        );
    }

    #[test]
    fn assistant_evidence_truncation_preserves_utf8_boundaries() {
        let text = format!("{}é", "x".repeat(MAX_ASSISTANT_TEXT_BYTES));
        let truncated = truncate_utf8(&text, MAX_ASSISTANT_TEXT_BYTES + 1);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert!(truncated.len() <= MAX_ASSISTANT_TEXT_BYTES + 1);
    }

    #[test]
    fn device_transcript_previews_are_bounded_on_utf8_boundaries() {
        let text = format!("{}é", "x".repeat(MAX_DEVICE_TRANSCRIPT_TEXT_BYTES));
        let preview = device_transcript_preview(&text);
        assert!(preview.is_char_boundary(preview.len()));
        assert!(preview.len() <= MAX_DEVICE_TRANSCRIPT_TEXT_BYTES);
        assert!(preview.contains("Transcript preview truncated"));
    }

    #[test]
    fn a_streamed_assistant_suffix_advances_the_device_projection() {
        let disclosure = omega_effectd::ExecutorDisclosure {
            executor_id: CLAUDE_AGENT_ID.to_string(),
            executor_name: "Claude Code".to_string(),
            model_id: Some("claude-opus".to_string()),
            model_name: Some("claude-opus".to_string()),
        };
        let previous = omega_effectd::MirrorThread {
            thread_ref: "thread.1".to_string(),
            title: "Projection".to_string(),
            executor: disclosure.clone(),
            state: omega_effectd::ThreadState::Running,
            transcript: vec![omega_effectd::MirrorMessage {
                message_ref: "message.1".to_string(),
                role: omega_effectd::MessageRole::Assistant,
                text: "Hello".to_string(),
                created_at: 1,
            }],
            updated_at: 1,
        };
        let mut current = previous.clone();
        current.transcript[0].text = "Hello world".to_string();
        current.updated_at = 2;
        let mut snapshot = omega_effectd::MirrorSnapshot::empty("Omega", 1, 1);
        snapshot.threads.push(previous.clone());
        let journal = omega_effectd::ProjectionJournal::new(snapshot);

        publish_device_transcript_delta(&journal, &previous, &current).expect("streaming suffix");

        let projected = journal.snapshot().expect("projected snapshot");
        assert_eq!(projected.sequence, 1);
        assert_eq!(projected.threads[0].transcript.len(), 2);
        assert_eq!(projected.threads[0].transcript[1].text, " world");
        assert_eq!(projected.threads[0].executor, disclosure);
    }

    #[test]
    fn a_large_initial_projection_is_reduced_to_one_transport_frame() {
        let disclosure = omega_effectd::ExecutorDisclosure {
            executor_id: "omega-agent".to_string(),
            executor_name: "Omega".to_string(),
            model_id: None,
            model_name: None,
        };
        let mut snapshot = omega_effectd::MirrorSnapshot::empty("Omega", 1, 1);
        for thread_index in 0..8 {
            snapshot.threads.push(omega_effectd::MirrorThread {
                thread_ref: format!("thread.{thread_index}"),
                title: format!("Thread {thread_index}"),
                executor: disclosure.clone(),
                state: omega_effectd::ThreadState::Running,
                transcript: (0..16)
                    .map(|message_index| omega_effectd::MirrorMessage {
                        message_ref: format!("message.{thread_index}.{message_index}"),
                        role: omega_effectd::MessageRole::Assistant,
                        text: "x".repeat(4 * 1024),
                        created_at: 1,
                    })
                    .collect(),
                updated_at: 1,
            });
        }

        let bounded = bound_device_mirror_snapshot(snapshot).expect("bounded snapshot");

        assert!(
            serde_json::to_vec(&bounded).expect("snapshot bytes").len()
                <= omega_effectd::MAX_FRAME_BYTES - DEVICE_SNAPSHOT_FRAME_RESERVE_BYTES
        );
        assert!(!bounded.threads.is_empty());
    }

    #[test]
    fn evidence_keys_remain_bounded_for_maximum_turn_refs() {
        let turn_ref = format!("{}é", "x".repeat(178));
        assert_eq!(turn_ref.len(), 180);

        let user_key = message_key(&turn_ref, "user");
        let assistant_key = message_key(&turn_ref, "assistant");

        assert!(user_key.len() <= 180);
        assert!(assistant_key.len() <= 180);
        assert_ne!(user_key, assistant_key);
        assert_eq!(user_key, message_key(&turn_ref, "user"));
    }

    #[test]
    fn correlation_journal_round_trips_without_conversation_entities() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: "workspace.omega.supervised".to_string(),
                lane: CLAUDE_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.1".to_string(),
                thread_id,
                conversation: None,
                turns: vec![HostTurn {
                    turn_ref: "turn.full-auto.1".to_string(),
                    lane: CODEX_LOCAL_LANE.to_string(),
                    account_ref: Some("account.owner".to_string()),
                    model: None,
                    provider_session_ref: "session.native.1".to_string(),
                    start_entry_index: 4,
                    end_entry_index: Some(8),
                    phase: "completed".to_string(),
                    disposition: Some("completed".to_string()),
                    created_at: "2026-07-24T12:00:00Z".to_string(),
                    updated_at: "2026-07-24T12:01:00Z".to_string(),
                }],
                revision: 3,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
            device_bridge: None,
            device_projection: None,
        };

        persist_correlation_journal(&state).expect("persist correlation journal");
        let restored = load_correlation_journal(&path).expect("load correlation journal");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].thread_id, thread_id);
        assert_eq!(restored[0].lane, CLAUDE_LOCAL_LANE);
        assert_eq!(restored[0].operation_ref, "operation.full-auto.1");
        assert!(restored[0].conversation.is_none());
        assert_eq!(restored[0].revision, 3);
        assert_eq!(restored[0].turns[0].turn_ref, "turn.full-auto.1");
        assert_eq!(
            restored[0].turns[0].disposition.as_deref(),
            Some("completed")
        );
    }

    /// OMEGA-DELTA-0021, omega#77's exit: a thread still names its executor
    /// after a restart.
    ///
    /// The restart is real rather than mimed. The lane index is process state,
    /// so it is emptied here exactly as a process exit empties it, and the only
    /// thing that survives into the assertion is the file on disk. A disclosure
    /// that lived only in memory fails this test at the `engine_lane_run` call.
    #[test]
    fn a_restarted_process_still_discloses_the_lane_that_owns_a_thread() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: SUPERVISED_WORKSPACE_REF.to_string(),
                lane: CODEX_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.77".to_string(),
                thread_id,
                conversation: None,
                turns: Vec::new(),
                revision: 1,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
            device_bridge: None,
            device_projection: None,
        };
        persist_correlation_journal(&state).expect("persist correlation journal");

        // The process ends here. Everything below is a cold start.
        republish_engine_lane_runs(&[]);
        assert_eq!(
            engine_lane_run(thread_id),
            None,
            "a cold process must know nothing until it reads the journal"
        );

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        republish_engine_lane_runs(&restored);

        let run = engine_lane_run(thread_id).expect("the reloaded journal names this thread's run");
        assert_eq!(run, "operation.full-auto.77");

        let disclosure = crate::omega_executor_disclosure::delegated_to_run(
            ExecutorDisclosure {
                class: ExecutorClass::ExternalAcp,
                agent_id: CODEX_AGENT_ID.to_string(),
                provider: None,
                model: None,
                run_ref: None,
                route: Some(omega_front_door::RouteReason::PinHonored),
            },
            run,
        );
        assert_eq!(disclosure.class, ExecutorClass::EngineLane);
        assert!(disclosure.is_coherent());

        let line = disclosure.label();
        assert!(line.contains(CODEX_AGENT_ID), "{line:?}");
        // omega#100. The class is asserted on the record above, not on the
        // line: the wire token is no longer rendered. What survives here is
        // the property the test is named for — a restarted process still
        // discloses the run that owns the thread.
        assert!(line.contains("operation.full-auto.77"), "{line:?}");
    }

    /// A thread the user started themselves is not a lane run, and must not be
    /// disclosed as one. Without this, the index's default answer would
    /// silently attribute every hand-driven thread to Full Auto.
    #[test]
    fn a_thread_that_is_not_a_lane_run_is_not_disclosed_as_one() {
        let _lane_index = lane_index_guard();
        let hand_started = ThreadId::new();
        republish_engine_lane_runs(&[HostThread {
            workspace_ref: SUPERVISED_WORKSPACE_REF.to_string(),
            lane: CLAUDE_LOCAL_LANE.to_string(),
            operation_ref: "operation.full-auto.1".to_string(),
            thread_id: ThreadId::new(),
            conversation: None,
            turns: Vec::new(),
            revision: 1,
        }]);
        assert_eq!(engine_lane_run(hand_started), None);
    }

    #[test]
    fn completed_turn_is_persisted_after_releasing_mutable_state_borrow() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = Rc::new(RefCell::new(HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: "workspace.omega.supervised".to_string(),
                lane: CODEX_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.1".to_string(),
                thread_id,
                conversation: None,
                turns: vec![HostTurn {
                    turn_ref: "turn.full-auto.1".to_string(),
                    lane: CODEX_LOCAL_LANE.to_string(),
                    account_ref: None,
                    model: None,
                    provider_session_ref: "session.native.1".to_string(),
                    start_entry_index: 2,
                    end_entry_index: None,
                    phase: "streaming".to_string(),
                    disposition: None,
                    created_at: "2026-07-24T12:00:00Z".to_string(),
                    updated_at: "2026-07-24T12:00:00Z".to_string(),
                }],
                revision: 1,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
            device_bridge: None,
            device_projection: None,
        }));

        record_turn_completion(&state, "turn.full-auto.1", 7, HostTurnOutcome::Completed)
            .expect("record completed turn");

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        let turn = &restored[0].turns[0];
        assert_eq!(restored[0].revision, 2);
        assert_eq!(turn.phase, "completed");
        assert_eq!(turn.disposition.as_deref(), Some("completed"));
        assert_eq!(turn.end_entry_index, Some(7));
    }

    #[test]
    fn claude_turns_have_a_bounded_host_deadline() {
        assert_eq!(
            turn_timeout_for_lane(CLAUDE_LOCAL_LANE),
            Some(CLAUDE_TURN_TIMEOUT)
        );
        assert_eq!(turn_timeout_for_lane(CODEX_LOCAL_LANE), None);
    }

    #[test]
    fn timed_out_turn_is_persisted_as_a_terminal_failure() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = Rc::new(RefCell::new(HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: "workspace.omega.supervised".to_string(),
                lane: CLAUDE_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.timeout".to_string(),
                thread_id,
                conversation: None,
                turns: vec![HostTurn {
                    turn_ref: "turn.full-auto.timeout".to_string(),
                    lane: CLAUDE_LOCAL_LANE.to_string(),
                    account_ref: None,
                    model: None,
                    provider_session_ref: "session.native.timeout".to_string(),
                    start_entry_index: 2,
                    end_entry_index: None,
                    phase: "streaming".to_string(),
                    disposition: None,
                    created_at: "2026-07-24T12:00:00Z".to_string(),
                    updated_at: "2026-07-24T12:00:00Z".to_string(),
                }],
                revision: 1,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
            device_bridge: None,
            device_projection: None,
        }));

        record_turn_completion(
            &state,
            "turn.full-auto.timeout",
            5,
            HostTurnOutcome::TimedOut,
        )
        .expect("record timed-out turn");

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        let turn = &restored[0].turns[0];
        assert_eq!(turn.phase, "failed");
        assert_eq!(turn.disposition.as_deref(), Some("timed_out"));
        assert_eq!(turn.end_entry_index, Some(5));
    }

    #[test]
    fn correlation_journal_rejects_unknown_schema_and_invalid_thread_refs() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "openagents.omega.full_auto_host_correlation.v0",
                "threads": [],
            }))
            .expect("serialize fixture"),
        )
        .expect("write fixture");
        assert!(load_correlation_journal(&path).is_err());

        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": CORRELATION_SCHEMA,
                "threads": [{
                    "workspaceRef": "workspace.omega.supervised",
                    "lane": "codex-local",
                    "operationRef": "operation.full-auto.1",
                    "threadRef": "not-a-uuid",
                    "turns": [],
                    "revision": 1,
                }],
            }))
            .expect("serialize fixture"),
        )
        .expect("write fixture");
        assert!(load_correlation_journal(&path).is_err());
    }

    #[test]
    fn local_lanes_resolve_to_their_real_agent_authorities() {
        assert_eq!(
            agent_for_lane(CODEX_LOCAL_LANE).expect("codex lane"),
            Agent::Custom {
                id: CODEX_AGENT_ID.into(),
            }
        );
        assert_eq!(
            agent_for_lane(CLAUDE_LOCAL_LANE).expect("claude lane"),
            Agent::Custom {
                id: CLAUDE_AGENT_ID.into(),
            }
        );
        assert!(agent_for_lane("claude-cloud").is_err());
    }

    #[test]
    fn external_agent_readiness_allows_dispatch_to_bootstrap_its_connection() {
        assert_eq!(
            external_agent_authority_state(false, AgentConnectionStatus::Disconnected, false,),
            (false, "unavailable")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Disconnected, false,),
            (true, "available")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Connecting, true),
            (true, "available")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Connecting, false),
            (false, "connecting")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Connected, true),
            (true, "available")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Disconnected, true),
            (true, "available")
        );
    }

    /// The environment a shipped app actually has: none.
    fn no_overrides() -> impl Fn(&str) -> Option<String> {
        |_| None
    }

    fn overrides(entries: &[(&str, &str)]) -> Box<dyn Fn(&str) -> Option<String>> {
        let entries = entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<StdHashMap<_, _>>();
        Box::new(move |name: &str| entries.get(name).cloned())
    }

    #[test]
    fn pairing_configuration_is_complete_with_an_empty_environment() {
        let lookup = no_overrides();
        assert_eq!(
            resolve_relay_urls(&lookup),
            vec!["wss://relay.openagents.com".to_owned()]
        );
        assert_eq!(
            resolve_sarah_public_key_hex(&lookup),
            "bcf86577b45042c960c99fe4ac1380a3ef0565ccbdd5c81e3f20f0919fe4fd14"
        );
        // Pin the "no Tailscale" path so a live daemon on the developer machine
        // cannot make this test advertise a real MagicDNS name.
        assert_eq!(resolve_bind_address_with(&lookup, None), "127.0.0.1");
        let endpoint = resolve_direct_device_endpoint_with(&lookup, None)
            .expect("an empty environment resolves the loopback endpoint");
        assert_eq!(endpoint.magic_dns_name, "localhost");
        assert_eq!(endpoint.port, 4317);
        assert_eq!(
            endpoint.protocol,
            omega_effectd::DEVICE_BRIDGE_PROTOCOL.to_string()
        );
        assert!(
            omega_effectd::BridgeBindHost::new(&resolve_bind_address_with(&lookup, None)).is_ok(),
            "the default bind address must satisfy the OMEGA-DELTA-0154 bind rule"
        );
    }

    /// A phone cannot dial `localhost`. When Tailscale reports a MagicDNS
    /// name and a CGNAT IPv4, those become the product default so Pair phone
    /// works with zero environment variables on a machine that is already
    /// on the tailnet.
    #[test]
    fn a_live_tailnet_becomes_the_default_pairing_endpoint() {
        let lookup = no_overrides();
        let live = LiveTailnetEndpoint {
            magic_dns_name: "macbook-pro-m5.tailaeab8f.ts.net".into(),
            bind_address: "100.127.107.31".into(),
        };
        assert_eq!(
            resolve_bind_address_with(&lookup, Some(&live)),
            "100.127.107.31"
        );
        let endpoint = resolve_direct_device_endpoint_with(&lookup, Some(&live))
            .expect("a live tailnet resolves the phone-reachable endpoint");
        assert_eq!(endpoint.magic_dns_name, "macbook-pro-m5.tailaeab8f.ts.net");
        assert_eq!(endpoint.port, 4317);
        assert!(
            omega_effectd::BridgeBindHost::new(&resolve_bind_address_with(&lookup, Some(&live)))
                .is_ok(),
            "the live CGNAT bind must satisfy the OMEGA-DELTA-0154 bind rule"
        );
    }

    #[test]
    fn live_tailnet_is_parsed_from_tailscale_status_json() {
        let json = br#"{
            "Self": {
                "DNSName": "macbook-pro-m5.tailaeab8f.ts.net.",
                "TailscaleIPs": ["100.127.107.31", "fd7a:115c:a1e0::4837:6b1f"]
            }
        }"#;
        assert_eq!(
            live_tailnet_from_status_json(json),
            Some(LiveTailnetEndpoint {
                magic_dns_name: "macbook-pro-m5.tailaeab8f.ts.net".into(),
                bind_address: "100.127.107.31".into(),
            })
        );
        assert_eq!(
            live_tailnet_from_status_json(br#"{"Self":{"DNSName":"x.ts.net.","TailscaleIPs":[]}}"#),
            None,
            "no IPv4 CGNAT address means no phone-reachable bind"
        );
        assert_eq!(
            live_tailnet_from_status_json(br#"{}"#),
            None,
            "a missing Self node is not a host to advertise"
        );
    }

    /// A fresh install has admitted no phone, and the QR flow is what admits
    /// the first one. An invented placeholder key here would be a device
    /// nobody holds sitting in an allowlist.
    #[test]
    fn a_fresh_install_admits_no_device_and_grants_owner_thread_commands() {
        let lookup = no_overrides();
        assert!(resolve_admitted_device_public_key_hexes(&lookup).is_empty());
        assert_eq!(
            resolve_approved_device_scopes(&lookup).expect("the built-in scope set parses"),
            vec![
                omega_effectd::Issue31PairingScope::ObserveIssue31,
                omega_effectd::Issue31PairingScope::SendMessage,
            ]
        );
    }

    #[test]
    fn default_phone_scope_is_limited_to_observation_and_agent_messages() {
        assert_eq!(
            DEFAULT_DEVICE_SCOPES,
            &[
                omega_effectd::Issue31PairingScope::ObserveIssue31,
                omega_effectd::Issue31PairingScope::SendMessage,
            ]
        );
    }

    #[test]
    fn the_environment_still_overrides_every_default() {
        let lookup = overrides(&[
            (
                "OPENAGENTS_OMEGA_NOSTR_RELAYS",
                "wss://one.test,wss://two.test",
            ),
            (
                "OPENAGENTS_OMEGA_SARAH_PUBLIC_KEY_HEX",
                "ab".repeat(32).as_str(),
            ),
            (
                "OPENAGENTS_OMEGA_NOSTR_DEVICE_PUBLIC_KEYS",
                "cd".repeat(32).as_str(),
            ),
            (
                "OPENAGENTS_OMEGA_NOSTR_DEVICE_SCOPES",
                "observe_issue31,send_message",
            ),
            (
                "OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS",
                "desk.tailnet.ts.net",
            ),
            ("OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT", "5900"),
            ("OPENAGENTS_OMEGA_DEVICE_BRIDGE_BIND_ADDRESS", "100.64.0.7"),
        ]);
        assert_eq!(
            resolve_relay_urls(&lookup),
            vec!["wss://one.test".to_owned(), "wss://two.test".to_owned()]
        );
        assert_eq!(resolve_sarah_public_key_hex(&lookup), "ab".repeat(32));
        assert_eq!(
            resolve_admitted_device_public_key_hexes(&lookup),
            vec!["cd".repeat(32)]
        );
        assert_eq!(
            resolve_approved_device_scopes(&lookup).expect("the override parses"),
            vec![
                omega_effectd::Issue31PairingScope::ObserveIssue31,
                omega_effectd::Issue31PairingScope::SendMessage,
            ]
        );
        assert_eq!(resolve_bind_address(&lookup), "100.64.0.7");
        let endpoint = resolve_direct_device_endpoint(&lookup).expect("the override resolves");
        assert_eq!(endpoint.magic_dns_name, "desk.tailnet.ts.net");
        assert_eq!(endpoint.port, 5900);
    }

    /// A blank export is the shape a launch script produces when a variable it
    /// meant to set was itself unset. Treating it as a value would put the
    /// shipped app back where it started.
    #[test]
    fn a_blank_override_falls_back_to_the_default() {
        let lookup = overrides(&[
            ("OPENAGENTS_OMEGA_NOSTR_RELAYS", "  "),
            ("OPENAGENTS_OMEGA_SARAH_PUBLIC_KEY_HEX", ""),
        ]);
        assert_eq!(
            resolve_relay_urls(&lookup),
            vec!["wss://relay.openagents.com".to_owned()]
        );
        assert_eq!(
            resolve_sarah_public_key_hex(&lookup),
            DEFAULT_SARAH_PUBLIC_KEY_HEX
        );
    }

    #[test]
    fn half_an_endpoint_override_names_the_missing_half() {
        // Pin no-live-tailnet so the refusal names the loopback built-in.
        let magic_dns_only = resolve_direct_device_endpoint_with(
            &overrides(&[(
                "OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS",
                "desk.tailnet.ts.net",
            )]),
            None,
        )
        .expect_err("advertising a name with no port is refused");
        assert!(
            magic_dns_only
                .message
                .contains("OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT is not")
                && magic_dns_only.message.contains("localhost:4317"),
            "the refusal must name the missing half and the built-in it replaces: {}",
            magic_dns_only.message
        );

        let port_only = resolve_direct_device_endpoint_with(
            &overrides(&[("OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT", "5900")]),
            None,
        )
        .expect_err("advertising a port with no name is refused");
        assert!(
            port_only
                .message
                .contains("OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS is not"),
            "the refusal must name the missing half: {}",
            port_only.message
        );

        for bad_port in ["0", "not-a-port", "99999"] {
            let refusal = resolve_direct_device_endpoint_with(
                &overrides(&[
                    (
                        "OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS",
                        "desk.tailnet.ts.net",
                    ),
                    ("OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT", bad_port),
                ]),
                None,
            )
            .expect_err("an unusable port is refused");
            assert!(
                refusal.message.contains(bad_port),
                "the refusal must quote the rejected port: {}",
                refusal.message
            );
        }
    }
}
