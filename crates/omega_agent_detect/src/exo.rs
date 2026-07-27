//! Where Exo is, and what a lane to it would have to say.
//!
//! `OMEGA-DELTA-0092`, omega#100. `OMEGA-DELTA-0055` made an unpinned thread
//! route to the external ACP agent that is attached, and then observed that
//! nothing attaches one: a thread reaches Exo only when a lane already exists,
//! and a lane existed only because somebody wrote
//! `omega-exo-lane.json` by hand. This module removes the hand.
//!
//! # Why Exo is not found the way the other agents are
//!
//! The rest of this crate looks for executables on `PATH`, which is what
//! `codex`, `claude`, `copilot` and `cursor-agent` are. Exo is not. It has no
//! release artifact at all — its install path is `curl setup.sh | bash`, which
//! *clones and builds from source* — so on this machine `exo` is not on `PATH`
//! and never will be. What exists is a checkout with a binary under its own
//! `target/`.
//!
//! That difference is not an inconvenience to route around. [`ExoLaneConfig`]
//! needs the checkout *and* the binary, and its own documentation says why: the
//! checkout is "the checkout the binary was built from, for the pin check". A
//! binary found on `PATH` would carry no evidence about which checkout built
//! it, so pairing it with a checkout found somewhere else would fabricate
//! exactly the correspondence the pin check assumes. **`PATH` is therefore not
//! searched for `exo`, on purpose**, and the pair is only ever taken from one
//! place: a checkout, and the `target/` directory inside it.
//!
//! [`ExoLaneConfig`]: ../../agent_ui/omega_exo_connection/struct.ExoLaneConfig.html
//!
//! # What is refused rather than guessed
//!
//! `OMEGA-DELTA-0042` exists because a half-read configuration points a lane at
//! the wrong `.exo`, and omega#86 was a day spent on the wrong Exo entirely —
//! exo labs' `exo-explore/exo` cluster-inference appliance shares nothing with
//! this one but a name. So every step here either produces the field or
//! produces [`ExoLaneUnderivable`], which names the field that is missing and
//! the exact path it was looked for at. Nothing is defaulted, nothing is
//! substituted, and an ambiguous answer is reported as ambiguous with the
//! candidates listed rather than resolved by picking the first one.
//!
//! # Why this reads files instead of asking Exo
//!
//! Asking would mean starting `exo agent list` on the startup path, which costs
//! a process launch before the window draws, and needs `EXO_SECRET_BACKEND` and
//! `EXO_MASTER_KEY_PATH` set correctly or Exo dies with a decryption error that
//! reads like a corrupt state root and is not. Reading is cheaper and quieter.
//!
//! The cost of reading is a coupling to Exo's on-disk layout, and the pin is
//! what makes that coupling safe: `EXO_PIN` names an exact commit and tree, so
//! the layout is pinned with everything else. This module checks only the
//! *upstream* — is this our Exo or the other one — and leaves the commit, the
//! tree and the bytes to the checks `OMEGA-DELTA-0042` already runs immediately
//! before every turn. That split is deliberate. Derivation answers "which
//! install", the pin answers "may it run"; a derivation that also enforced the
//! commit would turn an actionable refusal on the disclosure line into a silent
//! "no lane found" for anyone whose checkout had moved.

use std::path::{Path, PathBuf};

use omega_exo_lane::EXO_PIN;

/// Directories under `$HOME` an Exo checkout is looked for in, in order.
///
/// A closed list rather than a scan. Walking a home directory to find a
/// checkout would read everything a person owns to answer one question, and
/// `OMEGA-DELTA-0054` already refused to open `$HOME` as a project for the same
/// reason. Anyone whose checkout is elsewhere names it with
/// [`CHECKOUT_ENV_VAR`], and the refusal says so.
pub const CHECKOUT_DIRECTORIES: &[&str] = &[
    "work/exo",
    "exo",
    "code/exo",
    "src/exo",
    "dev/exo",
    "Developer/exo",
];

/// Names the Exo checkout when it is not in one of [`CHECKOUT_DIRECTORIES`].
pub const CHECKOUT_ENV_VAR: &str = "OMEGA_EXO_CHECKOUT";

/// Names Exo's state root when it is not `<checkout>/.exo`.
pub const ROOT_ENV_VAR: &str = "OMEGA_EXO_ROOT";

/// Names a lane file whose `root` is worth trying.
///
/// The spelling `crates/agent_ui`'s own live-Exo test already documents. A
/// machine that has been pointed at an Exo once has written this down
/// somewhere, and the root in it is a better guess than any directory this
/// module could invent.
pub const LANE_FILE_ENV_VAR: &str = "OMEGA_EXO_LANE_FILE";

/// The schema a lane file must carry before its `root` is believed.
///
/// Duplicated from `ExoLaneConfig`, deliberately and narrowly: this module
/// reads two fields out of that file and leaves the other four to the type that
/// owns the format. `OMEGA-DELTA-0092`'s check asserts the two spellings agree,
/// because a schema guard that silently stopped matching would make this read
/// accept a file the product refuses.
pub const LANE_FILE_SCHEMA: &str = "openagents.omega.exo_lane.v1";

/// Names the agent slug when the state root holds more than one.
pub const AGENT_ENV_VAR: &str = "OMEGA_EXO_AGENT";

/// Names the conversation slug when the agent holds more than one.
pub const CONVERSATION_ENV_VAR: &str = "OMEGA_EXO_CONVERSATION";

/// The state root's name inside the checkout.
///
/// Exo's own `--root` default is the relative path `.exo`, resolved against the
/// working directory the CLI was started in, and `exo` is started from its
/// checkout. So `<checkout>/.exo` is Exo's own default rather than a location
/// Omega invented.
pub const STATE_ROOT_DIRECTORY: &str = ".exo";

/// Exo's harness storage, one level *inside* the state root.
///
/// This is the trap this module was nearly written around. `--root` is not
/// where the records are: `crates/cli/src/main.rs` builds the harness with
/// `root: cli.root.join("exoharness")`, and the object store is opened with
/// that as its prefix. So an agent record is at
/// `<root>/exoharness/agents/<id>/record.json`, and a reader that looked for
/// `<root>/agents` would find nothing on a machine with agents and conclude
/// Exo had never run. `<root>` also holds `adapters/`, `adapters.lock` and
/// `exo-profile.md`, which are the CLI's and not the harness's — which is what
/// the extra level is for.
pub const HARNESS_DIRECTORY: &str = "exoharness";

/// Where a built `exo` is looked for inside a checkout, in order.
///
/// Release first because that is what `setup.sh` builds. A checkout holding
/// both gets the release binary, and whether *those bytes* may run is not this
/// module's decision: the digest check in `OMEGA-DELTA-0042` runs immediately
/// before every turn and is the authority.
pub const BINARY_PATHS: &[&str] = &["target/release/exo", "target/debug/exo"];

/// Everything an Exo lane needs, derived from what is on this machine.
///
/// The field set is [`ExoLaneConfig`]'s, deliberately: this type is what is
/// handed to it, and a field here that it does not have would be a field
/// nothing reads.
///
/// [`ExoLaneConfig`]: ../../agent_ui/omega_exo_connection/struct.ExoLaneConfig.html
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedExoLane {
    /// The `exo` binary.
    pub binary: PathBuf,
    /// The checkout it was built from.
    pub checkout: PathBuf,
    /// Exo's state root.
    pub root: PathBuf,
    /// The agent slug.
    pub agent: String,
    /// The conversation slug.
    pub conversation: String,
}

/// Which field of a lane could not be derived, and what was looked at.
///
/// Typed and per-field rather than one string, because the whole point is that
/// a caller can say which one is missing. "Exo is not installed", "that is the
/// other Exo", "Exo has never been run here" and "Exo has four agents and Omega
/// will not choose for you" are four different sentences and four different
/// things for a person to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoLaneUnderivable {
    /// No checkout at [`CHECKOUT_ENV_VAR`] or under [`CHECKOUT_DIRECTORIES`].
    NoCheckout {
        /// Every directory that was looked at, so the message can list them.
        searched: Vec<PathBuf>,
    },
    /// A checkout is there and it is not the Exo this lane drives.
    ///
    /// The omega#86 failure, named rather than skipped past. `upstream` is
    /// `None` when the remote could not be read at all, which is a different
    /// fact from reading a remote that names somebody else's repository.
    NotTheExoOmegaDrives {
        /// The directory that was rejected.
        checkout: PathBuf,
        /// The remote that was read, when one was.
        upstream: Option<String>,
    },
    /// The checkout is the right one and nothing has been built in it.
    NotBuilt {
        /// The checkout.
        checkout: PathBuf,
        /// The binary paths that were looked at.
        searched: Vec<PathBuf>,
    },
    /// No Exo state root at any candidate.
    ///
    /// The plural is the correction. This carried one `expected` path, the
    /// checkout's own `.exo`, and the summary drawn from it was "Exo has never
    /// been run on this machine" — while two roots with live agents sat
    /// elsewhere on the same disk. A refusal that names one place must not be
    /// read as a statement about every place, so it now names every place it
    /// looked.
    NoStateRoot {
        /// Every candidate, in the order they were tried.
        searched: Vec<PathBuf>,
    },
    /// The state root holds no agent.
    NoAgent {
        /// The state root.
        root: PathBuf,
    },
    /// The state root holds several agents and none was named.
    SeveralAgents {
        /// The state root.
        root: PathBuf,
        /// Every slug found, sorted, so the message is stable.
        slugs: Vec<String>,
    },
    /// The agent holds no conversation.
    NoConversation {
        /// The state root.
        root: PathBuf,
        /// The agent slug.
        agent: String,
    },
    /// The agent holds several conversations and none was named.
    SeveralConversations {
        /// The agent slug.
        agent: String,
        /// Every slug found, sorted.
        slugs: Vec<String>,
    },
}

impl std::fmt::Display for ExoLaneUnderivable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCheckout { searched } => write!(
                formatter,
                "no Exo checkout: looked at {}; set {CHECKOUT_ENV_VAR} to name one",
                display_paths(searched)
            ),
            Self::NotTheExoOmegaDrives { checkout, upstream } => match upstream {
                Some(upstream) => write!(
                    formatter,
                    "{} is a checkout of {upstream}, not {}",
                    checkout.display(),
                    EXO_PIN.upstream
                ),
                None => write!(
                    formatter,
                    "{} has no readable git remote, so it cannot be shown to be {}",
                    checkout.display(),
                    EXO_PIN.upstream
                ),
            },
            Self::NotBuilt { checkout, searched } => write!(
                formatter,
                "no exo binary built in {}: looked at {}",
                checkout.display(),
                display_paths(searched)
            ),
            Self::NoStateRoot { searched } => write!(
                formatter,
                "no Exo state root: looked at {}; set {ROOT_ENV_VAR} to name one",
                display_paths(searched)
            ),
            Self::NoAgent { root } => write!(
                formatter,
                "{} holds no Exo agent; Omega does not create one",
                root.display()
            ),
            Self::SeveralAgents { root, slugs } => write!(
                formatter,
                "{} holds {} Exo agents ({}); set {AGENT_ENV_VAR} to name one",
                root.display(),
                slugs.len(),
                slugs.join(", ")
            ),
            Self::NoConversation { root, agent } => write!(
                formatter,
                "the Exo agent {agent} in {} holds no conversation; Omega does \
                 not create one",
                root.display()
            ),
            Self::SeveralConversations { agent, slugs } => write!(
                formatter,
                "the Exo agent {agent} holds {} conversations ({}); set \
                 {CONVERSATION_ENV_VAR} to name one",
                slugs.len(),
                slugs.join(", ")
            ),
        }
    }
}

impl std::error::Error for ExoLaneUnderivable {}

/// What a caller already knows, so derivation does not have to find it.
///
/// Every field is a way of saying "not that one, this one". They are read from
/// the environment exactly once, in [`derive_lane_from_env`], for the same
/// reason `PATH` is a parameter everywhere else in this crate: the code that
/// decides what a shipped binary does on startup is the code no test in this
/// repository reaches, and a function that reads ambient process state can only
/// be tested on a machine that happens to be in the right state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExoLaneOverrides {
    /// The checkout, instead of searching [`CHECKOUT_DIRECTORIES`].
    pub checkout: Option<PathBuf>,
    /// The state root, instead of looking for one. Always wins.
    pub root: Option<PathBuf>,
    /// Where the process was started, when that is somewhere a person chose.
    ///
    /// A parameter and not a `current_dir()` call, for this module's usual
    /// reason, and for a sharper one: the working directory is the single fact
    /// that distinguishes the machine this derivation was first written against
    /// — where it found nothing — from the same machine a directory later,
    /// where it finds a working lane.
    ///
    /// See [`chosen_working_directory`] for why "somewhere a person chose" is
    /// not the same as "wherever the process was started".
    pub working_directory: Option<PathBuf>,
    /// A lane file whose `root` is worth trying. See [`LANE_FILE_ENV_VAR`].
    pub lane_file: Option<PathBuf>,
    /// The agent slug, instead of the only one there is.
    pub agent: Option<String>,
    /// The conversation slug, instead of the only one there is.
    pub conversation: Option<String>,
}

/// The checkout to use, given what the caller named and where `home` is.
///
/// A named checkout that is not our Exo is refused rather than searched past.
/// Someone who set [`CHECKOUT_ENV_VAR`] to the wrong directory is owed the
/// refusal, not a silent fall back to a different install.
///
/// # Errors
///
/// [`ExoLaneUnderivable::NoCheckout`] when nothing was found, or
/// [`ExoLaneUnderivable::NotTheExoOmegaDrives`] when a named one is the other
/// Exo.
pub fn find_checkout(
    named: Option<&Path>,
    home: &Path,
) -> Result<PathBuf, ExoLaneUnderivable> {
    if let Some(named) = named {
        return admit_checkout(named);
    }
    let searched: Vec<PathBuf> = CHECKOUT_DIRECTORIES
        .iter()
        .map(|directory| home.join(directory))
        .collect();
    // A directory that exists and names somebody else's repository is worth
    // more than the generic "nothing found": a person with exo labs'
    // `exo-explore/exo` cloned at `work/exo` is in the omega#86 situation, and
    // telling them Omega looked in six places and found nothing would send them
    // to install the thing they already have. A candidate that is simply absent
    // is not remembered — it is not evidence of anything.
    let mut wrong_exo = None;
    for candidate in &searched {
        match admit_checkout(candidate) {
            Ok(checkout) => return Ok(checkout),
            Err(refusal @ ExoLaneUnderivable::NotTheExoOmegaDrives {
                upstream: Some(_), ..
            }) => {
                wrong_exo.get_or_insert(refusal);
            }
            Err(_) => {}
        }
    }
    Err(wrong_exo.unwrap_or(ExoLaneUnderivable::NoCheckout { searched }))
}

/// Whether `checkout` is a checkout of the Exo this lane drives.
///
/// The remote is read out of `.git/config` rather than by running
/// `git config --get remote.origin.url`, because this runs before the window
/// draws and a process launch there is a cost paid on every start. The turn-time
/// check in `OMEGA-DELTA-0042` runs the real `git` and is the authority; this is
/// only the difference between two installs.
///
/// # Errors
///
/// [`ExoLaneUnderivable::NotTheExoOmegaDrives`], carrying the remote when there
/// was one to read.
pub fn admit_checkout(checkout: &Path) -> Result<PathBuf, ExoLaneUnderivable> {
    let refuse = |upstream: Option<String>| ExoLaneUnderivable::NotTheExoOmegaDrives {
        checkout: checkout.to_path_buf(),
        upstream,
    };
    if !checkout.is_dir() {
        return Err(refuse(None));
    }
    let config = std::fs::read_to_string(checkout.join(".git").join("config"))
        .map_err(|_| refuse(None))?;
    let mut refused: Option<String> = None;
    for url in git_config_urls(&config) {
        if EXO_PIN.admits_upstream(&url).is_ok() {
            return Ok(checkout.to_path_buf());
        }
        refused.get_or_insert(url);
    }
    Err(refuse(refused))
}

/// Every `url = ...` value in a `.git/config`, in file order.
///
/// All of them rather than `[remote "origin"]`'s, because a clone whose origin
/// is a fork and whose `upstream` remote is ours is still a checkout of ours,
/// and because parsing INI sections to find one key would be more machinery for
/// a worse answer. A checkout that names our repository under any remote is
/// ours.
fn git_config_urls(config: &str) -> impl Iterator<Item = String> + '_ {
    config.lines().filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "url").then(|| value.trim().to_owned())
    })
}

/// The `exo` built inside `checkout`.
///
/// # Errors
///
/// [`ExoLaneUnderivable::NotBuilt`], listing where it looked, when the checkout
/// is there and nothing in it has been built.
pub fn built_binary(checkout: &Path) -> Result<PathBuf, ExoLaneUnderivable> {
    let searched: Vec<PathBuf> = BINARY_PATHS
        .iter()
        .map(|relative| checkout.join(relative))
        .collect();
    searched
        .iter()
        .find(|candidate| crate::is_executable_file(candidate))
        .cloned()
        .ok_or_else(|| ExoLaneUnderivable::NotBuilt {
            checkout: checkout.to_path_buf(),
            searched,
        })
}

/// Exo's state root, given what the caller knows and which checkout was found.
///
/// # Where a root can be, and why it is not one place
///
/// Exo's root is wherever `--root` said. Its default is the relative path
/// `.exo`, resolved against the working directory the CLI was started in — so
/// the root lives beside *whatever directory somebody ran `exo` from*, which on
/// a real machine is very often not the checkout. The first version of this
/// function looked only beside the checkout, found nothing, and the absence was
/// reported as "Exo has never been run here" while two roots with live agents
/// sat elsewhere on the same disk.
///
/// That is the same mistake as reading `<root>/agents` instead of
/// `<root>/exoharness/agents`, one level further out: a reader that looks in
/// one place produces a confident false absence on a machine that has the
/// thing. Both are fixed the same way — look everywhere it could be, and name
/// everywhere you looked.
///
/// The order is [`ROOT_CANDIDATE_ORDER`]'s.
///
/// # Why a root with an agent wins
///
/// Among the candidates that exist, one holding at least one agent is preferred
/// over one that merely exists. An empty root is the same dead end as no root —
/// [`agent_slug`] refuses on it — so choosing it over a working one would
/// reintroduce the failure the search exists to remove. An explicitly named
/// root is exempt: the caller said which one, and quietly using a different
/// one because theirs looked emptier would be Omega disagreeing with an
/// instruction.
///
/// # Errors
///
/// [`ExoLaneUnderivable::NoStateRoot`], naming every candidate it tried.
pub fn state_root(
    overrides: &ExoLaneOverrides,
    checkout: &Path,
) -> Result<PathBuf, ExoLaneUnderivable> {
    if let Some(named) = overrides.root.as_deref() {
        return if is_state_root(named) {
            Ok(named.to_path_buf())
        } else {
            Err(ExoLaneUnderivable::NoStateRoot {
                searched: vec![named.to_path_buf()],
            })
        };
    }

    let searched: Vec<PathBuf> = [
        overrides
            .working_directory
            .as_ref()
            .map(|cwd| cwd.join(STATE_ROOT_DIRECTORY)),
        Some(checkout.join(STATE_ROOT_DIRECTORY)),
        overrides
            .lane_file
            .as_deref()
            .and_then(root_named_by_lane_file),
    ]
    .into_iter()
    .flatten()
    .collect();

    let present: Vec<&PathBuf> = searched
        .iter()
        .filter(|candidate| is_state_root(candidate))
        .collect();
    present
        .iter()
        .find(|candidate| !record_slugs(&agents_directory(candidate)).is_empty())
        .or(present.first())
        .map(|candidate| (*candidate).clone())
        .ok_or(ExoLaneUnderivable::NoStateRoot { searched })
}

/// The candidate order, as prose a reader can check the code against.
///
/// Named as a constant rather than left implicit in [`state_root`] because the
/// order *is* the policy: an explicit instruction, then the directory the
/// person is standing in, then the checkout, then a root somebody already wrote
/// down.
pub const ROOT_CANDIDATE_ORDER: &[&str] = &[
    "OMEGA_EXO_ROOT, which always wins",
    "<working directory>/.exo, which is Exo's own --root default",
    "<checkout>/.exo",
    "the root named by the lane file at OMEGA_EXO_LANE_FILE",
];

/// Whether a directory is an Exo state root.
///
/// The harness directory rather than the root, and rather than the agents
/// directory inside it. Exo's object store creates `<root>/exoharness` eagerly
/// when it opens, before any agent exists, and creates `agents/` only when
/// something is written into it. Testing for `agents/` would report "Exo has
/// never run here" about a root where Exo has run and simply holds no agent —
/// a different sentence with a different thing to do about it, and
/// [`agent_slug`] says it.
fn is_state_root(candidate: &Path) -> bool {
    candidate.join(HARNESS_DIRECTORY).is_dir()
}

/// The `root` a lane file names, if it carries the schema Omega understands.
///
/// Two fields, deliberately. The other four belong to `ExoLaneConfig`, which
/// owns the format; this is a search hint, and treating it as a whole lane
/// would mean a second parser for a file that already has one. The schema is
/// still checked, because a `root` read out of a file whose shape Omega does
/// not recognise is a path with no provenance.
fn root_named_by_lane_file(lane_file: &Path) -> Option<PathBuf> {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(lane_file).ok()?).ok()?;
    if value.get("schema").and_then(serde_json::Value::as_str) != Some(LANE_FILE_SCHEMA) {
        return None;
    }
    value
        .get("root")
        .and_then(serde_json::Value::as_str)
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
}

/// `<root>/exoharness/agents`, where Exo keeps one directory per agent.
///
/// See [`HARNESS_DIRECTORY`] for why the middle component is there.
fn agents_directory(root: &Path) -> PathBuf {
    root.join(HARNESS_DIRECTORY).join("agents")
}

/// The agent slug the lane sends to.
///
/// A named slug is taken as given: the caller is telling Omega which of several
/// agents it means, and re-deriving it would only be a way to disagree with
/// them.
///
/// # Errors
///
/// [`ExoLaneUnderivable::NoAgent`] when the root holds none, and
/// [`ExoLaneUnderivable::SeveralAgents`] when it holds more than one — listed,
/// not chosen between. Sending somebody's first message to whichever agent a
/// directory listing happened to yield first is the "pointed at the wrong one"
/// failure with a smaller radius, not a different failure.
pub fn agent_slug(named: Option<&str>, root: &Path) -> Result<String, ExoLaneUnderivable> {
    if let Some(named) = named {
        return Ok(named.to_owned());
    }
    let mut slugs = record_slugs(&agents_directory(root));
    match slugs.len() {
        0 => Err(ExoLaneUnderivable::NoAgent {
            root: root.to_path_buf(),
        }),
        1 => Ok(slugs.remove(0)),
        _ => Err(ExoLaneUnderivable::SeveralAgents {
            root: root.to_path_buf(),
            slugs,
        }),
    }
}

/// The conversation slug the lane sends to, inside `agent`.
///
/// # Errors
///
/// [`ExoLaneUnderivable::NoConversation`] and
/// [`ExoLaneUnderivable::SeveralConversations`], for the reasons
/// [`agent_slug`] gives.
pub fn conversation_slug(
    named: Option<&str>,
    root: &Path,
    agent: &str,
) -> Result<String, ExoLaneUnderivable> {
    if let Some(named) = named {
        return Ok(named.to_owned());
    }
    let Some(directory) = agent_directory(root, agent) else {
        return Err(ExoLaneUnderivable::NoConversation {
            root: root.to_path_buf(),
            agent: agent.to_owned(),
        });
    };
    let mut records = conversation_records(&directory.join("conversations"));
    if records.is_empty() {
        return Err(ExoLaneUnderivable::NoConversation {
            root: root.to_path_buf(),
            agent: agent.to_owned(),
        });
    }
    if records.len() == 1 {
        return Ok(records.remove(0).slug);
    }
    // Several. Ordered by evidence rather than refused, and the difference from
    // `agent_slug` is the point.
    //
    // Two agents are two different capabilities: different tool modules,
    // different mounts, a different model binding. Choosing between them is the
    // "pointed at the wrong one" failure `OMEGA-DELTA-0042` exists for, so it
    // stays a refusal. Two conversations are two threads of the *same* agent,
    // with the same capability and the same mounts; the worst case is that a
    // message lands in a thread the person was not looking at, which is visible
    // the moment it happens.
    //
    // So the tie is broken by what the agent was last used for.
    // `latest_event_id` is a UUIDv7, which is time-ordered by construction, so
    // "most recent" is a string comparison over a value Exo already wrote — not
    // a file mtime, which a copy or a backup rewrites.
    //
    // A tie with no evidence is still a refusal: conversations that have never
    // been used give nothing to order them by, and picking one would be the
    // guess this module does not make.
    let latest = records
        .iter()
        .filter_map(|record| {
            record
                .latest_event_id
                .as_ref()
                .map(|event| (event.clone(), record.slug.clone()))
        })
        .max();
    match latest {
        Some((_, slug)) => Ok(slug),
        None => {
            let mut slugs: Vec<String> = records.into_iter().map(|record| record.slug).collect();
            slugs.sort();
            Err(ExoLaneUnderivable::SeveralConversations {
                agent: agent.to_owned(),
                slugs,
            })
        }
    }
}

/// One conversation, as much of it as the choice above needs.
struct ConversationRecord {
    slug: String,
    /// Exo's own marker of the last thing that happened in it, when anything
    /// has. A UUIDv7, so it sorts by time.
    latest_event_id: Option<String>,
}

/// Every conversation directly under `directory`.
fn conversation_records(directory: &Path) -> Vec<ConversationRecord> {
    record_directories(directory)
        .into_iter()
        .map(|(path, slug)| ConversationRecord {
            slug,
            latest_event_id: record_field(&path.join("record.json"), "latest_event_id"),
        })
        .collect()
}

/// The directory holding the agent whose record carries `slug`.
///
/// Exo names agent directories by id, not by slug, so the slug is read out of
/// each `record.json`. The `by-slug` marker files next to them are not used:
/// their names are percent-encoded, so recovering a slug from one means undoing
/// an encoding, and a record that says what it is beats a filename that has to
/// be decoded into what it might be.
fn agent_directory(root: &Path, slug: &str) -> Option<PathBuf> {
    record_directories(&agents_directory(root))
        .into_iter()
        .find(|(_, record_slug)| record_slug == slug)
        .map(|(directory, _)| directory)
}

/// The slugs of every record directly under `directory`, sorted.
///
/// Sorted so a message listing them is the same on two machines, and so a test
/// asserting the list does not depend on the order a filesystem hands entries
/// back.
fn record_slugs(directory: &Path) -> Vec<String> {
    let mut slugs: Vec<String> = record_directories(directory)
        .into_iter()
        .map(|(_, slug)| slug)
        .collect();
    slugs.sort();
    slugs
}

/// Every `<directory>/<id>/record.json` that parses, as its directory and slug.
///
/// A record that cannot be read or does not carry a slug is skipped rather than
/// reported. It is not evidence of an agent — and it is also not evidence of
/// none, which is why nothing here concludes anything from an unreadable file
/// beyond leaving it out of the count.
fn record_directories(directory: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let slug = record_field(&path.join("record.json"), "slug")?;
            Some((path, slug))
        })
        .collect()
}

/// One string field of a record file, when it is there and is not blank.
fn record_field(record: &Path, field: &str) -> Option<String> {
    let file = std::fs::read_to_string(record).ok()?;
    let value: serde_json::Value = serde_json::from_str(&file).ok()?;
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/// The working directory, when it is one somebody chose to be in.
///
/// `OMEGA-DELTA-0092`, omega#100. The root search's second candidate is
/// `<working directory>/.exo`, because Exo's `--root` default is `.exo`
/// relative to the directory `exo` was run from. That is true of a person in a
/// shell and false of a launcher: macOS hands a Finder or Dock launch a working
/// directory of `/`, and a packaged Omega is started that way far more often
/// than from a terminal.
///
/// So an ungated read makes the candidate `/.exo` on exactly the launch a new
/// person makes — inert at best, and at worst a path nobody named offered to
/// the search that decides which `.exo` somebody's first message lands in.
/// Every other candidate here is either an explicit instruction or a location
/// tied to something the person set up; a launcher's `/` is neither.
///
/// The plausibility rule is [`omega_workdir::plausible_project_root`]'s and not
/// a second one written here. `OMEGA-DELTA-0054` already asks "is this a
/// directory a person chose" on this same startup path, to decide what the
/// thread's `grep` and `read_file` can see, and two answers to that question
/// would eventually disagree about the same launch — the thread opened on one
/// directory and its Exo lane derived from another.
///
/// Inputs are parameters for this module's usual reason: startup is the path no
/// test in this repository reaches.
#[must_use]
pub fn chosen_working_directory(
    working_directory: Option<PathBuf>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let working_directory = working_directory?;
    omega_workdir::plausible_project_root(&working_directory, home).ok()
}

fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A whole lane, or the first field that could not be derived.
///
/// The order is the order the fields depend on each other in: there is no root
/// without a checkout and no conversation without an agent, so the first
/// failure is always the most upstream one and the message is about the thing a
/// person would have to fix first.
///
/// # Errors
///
/// [`ExoLaneUnderivable`], naming that field.
pub fn derive_lane(
    overrides: &ExoLaneOverrides,
    home: &Path,
) -> Result<DerivedExoLane, ExoLaneUnderivable> {
    let checkout = find_checkout(overrides.checkout.as_deref(), home)?;
    let binary = built_binary(&checkout)?;
    let root = state_root(overrides, &checkout)?;
    let agent = agent_slug(overrides.agent.as_deref(), &root)?;
    let conversation = conversation_slug(overrides.conversation.as_deref(), &root, &agent)?;
    Ok(DerivedExoLane {
        binary,
        checkout,
        root,
        agent,
        conversation,
    })
}

/// [`derive_lane`] against the process's own environment.
///
/// The one place in this module that reads ambient state, and it reads nothing
/// else: no `PATH`, no working directory, no settings. A missing `$HOME` is
/// reported as no checkout rather than defaulting to `/`, because searching the
/// filesystem root for `work/exo` would be a guess dressed as a search.
///
/// # Errors
///
/// [`ExoLaneUnderivable`], as [`derive_lane`] does.
pub fn derive_lane_from_env() -> Result<DerivedExoLane, ExoLaneUnderivable> {
    let path = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let text = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| ExoLaneUnderivable::NoCheckout {
            searched: Vec::new(),
        })?;
    let overrides = ExoLaneOverrides {
        checkout: path(CHECKOUT_ENV_VAR),
        root: path(ROOT_ENV_VAR),
        // A working directory that cannot be read is treated as no working
        // directory, exactly as `omega_workdir` treats it: the other candidates
        // still stand, and guessing one here would put a lane somewhere nobody
        // named. A working directory a *launcher* chose is treated the same way
        // and for the same reason — see `chosen_working_directory`.
        working_directory: chosen_working_directory(std::env::current_dir().ok(), Some(&home)),
        lane_file: path(LANE_FILE_ENV_VAR),
        agent: text(AGENT_ENV_VAR),
        conversation: text(CONVERSATION_ENV_VAR),
    };
    derive_lane(&overrides, &home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A checkout that `admit_checkout` accepts, with `remote` as its origin.
    fn write_checkout(home: &Path, relative: &str, remote: &str) -> PathBuf {
        let checkout = home.join(relative);
        fs::create_dir_all(checkout.join(".git")).expect("the fixture checkout is created");
        fs::write(
            checkout.join(".git").join("config"),
            format!("[remote \"origin\"]\n\turl = {remote}\n\tfetch = +refs/heads/*\n"),
        )
        .expect("the fixture git config is written");
        checkout
    }

    fn write_binary(checkout: &Path, relative: &str) -> PathBuf {
        let path = checkout.join(relative);
        fs::create_dir_all(path.parent().expect("the binary has a parent"))
            .expect("the fixture target directory is created");
        fs::write(&path, "#!/bin/sh\n").expect("the fixture binary is written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = fs::metadata(&path)
                .expect("the fixture binary exists")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("the fixture binary is executable");
        }
        path
    }

    fn write_record(directory: &Path, slug: &str) {
        write_used_record(directory, slug, None);
    }

    /// A record with Exo's own marker of when it was last used.
    fn write_used_record(directory: &Path, slug: &str, latest_event_id: Option<&str>) {
        fs::create_dir_all(directory).expect("the fixture record directory is created");
        fs::write(
            directory.join("record.json"),
            serde_json::json!({
                "id": "0",
                "slug": slug,
                "name": slug,
                "latest_event_id": latest_event_id,
            })
            .to_string(),
        )
        .expect("the fixture record is written");
    }

    /// The directory Exo actually keeps agents in, spelled out once so every
    /// fixture below builds the same layout the harness does.
    fn agents(root: &Path) -> PathBuf {
        root.join(HARNESS_DIRECTORY).join("agents")
    }

    /// A whole machine: our Exo, built, with one agent and one conversation.
    fn write_machine(home: &Path) -> PathBuf {
        let checkout = write_checkout(home, "work/exo", EXO_PIN.upstream);
        write_binary(&checkout, "target/release/exo");
        let agents = agents(&checkout.join(STATE_ROOT_DIRECTORY));
        write_record(&agents.join("agent-id"), "omega");
        write_record(
            &agents
                .join("agent-id")
                .join("conversations")
                .join("conversation-id"),
            "basic",
        );
        checkout
    }

    /// The layout mistake this module was nearly built on.
    ///
    /// Exo's `--root` is not the harness's root: the CLI opens the object store
    /// at `<root>/exoharness`. A reader that looked one level too high would
    /// find no agents on a machine that has them, and would say Exo had never
    /// run here. This asserts the level, not the outcome, so it fails on the
    /// mistake rather than on a machine.
    #[test]
    fn agents_are_read_from_inside_the_harness_directory() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        let root = checkout.join(STATE_ROOT_DIRECTORY);

        assert_eq!(agents_directory(&root), root.join("exoharness").join("agents"));
        assert!(
            !root.join("agents").exists(),
            "the fixture must not also write the wrong layout, or this proves \
             nothing"
        );
        assert_eq!(
            agent_slug(None, &root).expect("the agent is found where Exo puts it"),
            "omega"
        );
    }

    #[test]
    fn a_whole_lane_is_derived_from_an_install() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());

        let lane = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect("every field is derivable from this machine");

        assert_eq!(lane.checkout, checkout);
        assert_eq!(lane.binary, checkout.join("target/release/exo"));
        assert_eq!(lane.root, checkout.join(".exo"));
        assert_eq!(lane.agent, "omega");
        assert_eq!(lane.conversation, "basic");
    }

    /// The omega#86 failure, which cost a day.
    #[test]
    fn the_other_exo_is_refused_by_name() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(
            home.path(),
            "work/exo",
            "https://github.com/exo-explore/exo",
        );
        write_binary(&checkout, "target/release/exo");

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("exo labs' appliance is not the Exo this lane drives");

        assert!(
            matches!(
                &refusal,
                ExoLaneUnderivable::NotTheExoOmegaDrives { upstream, .. }
                    if upstream.as_deref() == Some("https://github.com/exo-explore/exo")
            ),
            "{refusal:?}"
        );
        assert!(refusal.to_string().contains("exo-explore/exo"), "{refusal}");
    }

    #[test]
    fn a_named_checkout_that_is_the_other_exo_is_not_searched_past() {
        let home = tempfile::tempdir().expect("a temporary home");
        // Ours is exactly where the search would have found it.
        write_machine(home.path());
        let other = write_checkout(
            home.path(),
            "elsewhere/exo",
            "https://github.com/exo-explore/exo",
        );

        let refusal = derive_lane(
            &ExoLaneOverrides {
                checkout: Some(other.clone()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect_err("a named checkout is answered, not searched past");

        assert!(
            matches!(
                &refusal,
                ExoLaneUnderivable::NotTheExoOmegaDrives { checkout, .. } if checkout == &other
            ),
            "{refusal:?}"
        );
    }

    #[test]
    fn a_remote_spelled_over_ssh_is_the_same_repository() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(home.path(), "work/exo", "git@github.com:OpenAgentsInc/exo.git");
        write_binary(&checkout, "target/debug/exo");

        assert_eq!(
            admit_checkout(&checkout).expect("the ssh spelling names the same repository"),
            checkout
        );
    }

    #[test]
    fn nothing_installed_names_where_it_looked() {
        let home = tempfile::tempdir().expect("a temporary home");

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("an empty home has no Exo");

        assert!(
            matches!(&refusal, ExoLaneUnderivable::NoCheckout { searched }
                if searched.len() == CHECKOUT_DIRECTORIES.len()),
            "{refusal:?}"
        );
        assert!(
            refusal.to_string().contains("work/exo"),
            "the refusal must say where it looked: {refusal}"
        );
    }

    /// `exo` has no release artifact, so a checkout is routinely unbuilt.
    #[test]
    fn a_checkout_with_nothing_built_names_the_binary_it_wanted() {
        let home = tempfile::tempdir().expect("a temporary home");
        write_checkout(home.path(), "work/exo", EXO_PIN.upstream);

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("a checkout is not an install");

        assert!(
            matches!(&refusal, ExoLaneUnderivable::NotBuilt { searched, .. }
                if searched.len() == BINARY_PATHS.len()),
            "{refusal:?}"
        );
        assert!(
            refusal.to_string().contains("target/release/exo"),
            "{refusal}"
        );
    }

    /// The `OMEGA-DELTA-0042` failure this whole module is shaped by: a lane
    /// pointed at a `.exo` that is not there must not become a lane pointed at
    /// some other `.exo`.
    #[test]
    fn an_absent_state_root_is_named_and_not_substituted() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(home.path(), "work/exo", EXO_PIN.upstream);
        write_binary(&checkout, "target/release/exo");

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("Exo has never been run in this checkout");

        assert_eq!(
            refusal,
            ExoLaneUnderivable::NoStateRoot {
                searched: vec![checkout.join(".exo")]
            }
        );
    }

    #[test]
    fn a_state_root_with_no_agent_is_not_a_lane() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(home.path(), "work/exo", EXO_PIN.upstream);
        write_binary(&checkout, "target/release/exo");
        fs::create_dir_all(agents(&checkout.join(STATE_ROOT_DIRECTORY)))
            .expect("the fixture state root is created");

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("Omega does not create an Exo agent");

        assert!(matches!(refusal, ExoLaneUnderivable::NoAgent { .. }), "{refusal:?}");
    }

    #[test]
    fn several_agents_are_listed_rather_than_chosen_between() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        write_record(
            &agents(&checkout.join(STATE_ROOT_DIRECTORY)).join("second-id"),
            "another",
        );

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("two agents is not one agent");

        assert!(
            matches!(&refusal, ExoLaneUnderivable::SeveralAgents { slugs, .. }
                if slugs == &["another".to_owned(), "omega".to_owned()]),
            "{refusal:?}"
        );
        assert!(refusal.to_string().contains(AGENT_ENV_VAR), "{refusal}");
    }

    #[test]
    fn a_named_agent_resolves_an_ambiguous_root() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        let agents = agents(&checkout.join(STATE_ROOT_DIRECTORY));
        write_record(&agents.join("second-id"), "another");
        write_record(
            &agents
                .join("second-id")
                .join("conversations")
                .join("c"),
            "other-conversation",
        );

        let lane = derive_lane(
            &ExoLaneOverrides {
                agent: Some("another".into()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect("naming the agent is what the refusal asked for");

        assert_eq!(lane.agent, "another");
        assert_eq!(
            lane.conversation, "other-conversation",
            "the conversation must come from the agent that was named, not from \
             the other one"
        );
    }

    /// Several conversations that have never been used give nothing to choose
    /// between, so the choice is refused rather than invented.
    #[test]
    fn several_unused_conversations_are_listed_rather_than_chosen_between() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        write_record(
            &agents(&checkout.join(STATE_ROOT_DIRECTORY))
                .join("agent-id")
                .join("conversations")
                .join("second-id"),
            "another",
        );

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("two conversations, neither of them ever used");

        assert!(
            matches!(&refusal, ExoLaneUnderivable::SeveralConversations { slugs, .. }
                if slugs == &["another".to_owned(), "basic".to_owned()]),
            "{refusal:?}"
        );
    }

    /// The real `exo-lane` root holds three conversations on one agent, and
    /// refusing there would be a dead end on the machine this is meant to work
    /// on. Two threads of the same agent share its capability and its mounts,
    /// so the tie is broken by what Exo was last used for.
    #[test]
    fn the_conversation_last_used_is_the_one_the_lane_resumes() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        let conversations = agents(&checkout.join(STATE_ROOT_DIRECTORY))
            .join("agent-id")
            .join("conversations");
        // UUIDv7s, so the later one sorts second by construction.
        write_used_record(
            &conversations.join("conversation-id"),
            "basic",
            Some("019f9ec3-9e64-7230-86b8-abc9054c82a2"),
        );
        write_used_record(
            &conversations.join("second-id"),
            "recent",
            Some("019f9f6f-92de-7630-b8f8-309366dcd7e2"),
        );

        let lane = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect("an agent that has been used names the thread it was used in");

        assert_eq!(lane.conversation, "recent");
    }

    /// The correction this search exists for.
    ///
    /// A root beside the working directory, with a checkout whose own `.exo`
    /// does not exist. The first version of this module found nothing here and
    /// reported that Exo had never been run on the machine.
    #[test]
    fn a_root_beside_the_working_directory_is_found() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(home.path(), "work/exo", EXO_PIN.upstream);
        write_binary(&checkout, "target/release/exo");
        let elsewhere = home.path().join("somewhere-else");
        let agents = agents(&elsewhere.join(STATE_ROOT_DIRECTORY));
        write_record(&agents.join("id"), "zerobase");
        write_record(&agents.join("id").join("conversations").join("c"), "zb-proof");

        let lane = derive_lane(
            &ExoLaneOverrides {
                working_directory: Some(elsewhere.clone()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect("Exo's root is wherever --root said, not only beside the checkout");

        assert_eq!(lane.root, elsewhere.join(STATE_ROOT_DIRECTORY));
        assert_eq!(lane.agent, "zerobase");
        assert_eq!(lane.checkout, checkout, "the checkout is still the checkout");
    }

    /// A root that merely exists is the same dead end as no root.
    #[test]
    fn a_root_with_an_agent_beats_an_empty_one_that_comes_first() {
        let home = tempfile::tempdir().expect("a temporary home");
        // The checkout's own root has agents; the working directory's is empty.
        let checkout = write_machine(home.path());
        let empty = home.path().join("empty");
        fs::create_dir_all(empty.join(STATE_ROOT_DIRECTORY).join(HARNESS_DIRECTORY))
            .expect("the fixture empty root is created");

        let lane = derive_lane(
            &ExoLaneOverrides {
                working_directory: Some(empty),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect("the root with an agent answers");

        assert_eq!(
            lane.root,
            checkout.join(STATE_ROOT_DIRECTORY),
            "an earlier candidate that holds no agent must not win over a later \
             one that does, or the search reintroduces the dead end it exists \
             to remove"
        );
    }

    /// An explicit root is an instruction, not a candidate.
    #[test]
    fn a_named_empty_root_is_not_swapped_for_a_fuller_one() {
        let home = tempfile::tempdir().expect("a temporary home");
        write_machine(home.path());
        let empty = home.path().join("empty");
        fs::create_dir_all(empty.join(HARNESS_DIRECTORY)).expect("the fixture root is created");

        let refusal = derive_lane(
            &ExoLaneOverrides {
                root: Some(empty.clone()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect_err("the named root holds no agent");

        assert!(
            matches!(&refusal, ExoLaneUnderivable::NoAgent { root } if root == &empty),
            "Omega must not quietly use a different root than the one it was \
             told to use: {refusal:?}"
        );
    }

    /// A named root that is not there names itself in the refusal.
    ///
    /// Separate from the search case: that one lists candidates Omega chose,
    /// this one has to echo back the path the caller gave, or the message reads
    /// as though Omega looked nowhere.
    #[test]
    fn a_named_root_that_is_absent_is_the_one_the_refusal_names() {
        let home = tempfile::tempdir().expect("a temporary home");
        write_machine(home.path());
        let nowhere = home.path().join("not-a-root");

        let refusal = derive_lane(
            &ExoLaneOverrides {
                root: Some(nowhere.clone()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect_err("the named root does not exist");

        assert_eq!(
            refusal,
            ExoLaneUnderivable::NoStateRoot {
                searched: vec![nowhere.clone()]
            }
        );
        assert!(
            refusal.to_string().contains(&nowhere.display().to_string()),
            "{refusal}"
        );
    }

    #[test]
    fn a_lane_file_elsewhere_names_a_root_worth_trying() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(home.path(), "work/exo", EXO_PIN.upstream);
        write_binary(&checkout, "target/release/exo");
        let elsewhere = home.path().join("named-by-a-lane-file");
        let agents = agents(&elsewhere.join(STATE_ROOT_DIRECTORY));
        write_record(&agents.join("id"), "omega-lane");
        write_record(&agents.join("id").join("conversations").join("c"), "tier-a");
        let lane_file = home.path().join("omega-exo-lane.json");
        fs::write(
            &lane_file,
            serde_json::json!({
                "schema": LANE_FILE_SCHEMA,
                "binary": "/somewhere/exo",
                "checkout": "/somewhere",
                "root": elsewhere.join(STATE_ROOT_DIRECTORY).to_string_lossy(),
                "agent": "omega-lane",
                "conversation": "tier-a",
            })
            .to_string(),
        )
        .expect("the fixture lane file is written");

        let lane = derive_lane(
            &ExoLaneOverrides {
                lane_file: Some(lane_file),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect("a root somebody already wrote down is a candidate");

        assert_eq!(lane.root, elsewhere.join(STATE_ROOT_DIRECTORY));
        assert_eq!(
            lane.binary,
            checkout.join("target/release/exo"),
            "only the root is taken from the lane file; the binary still comes \
             from the checkout that built it"
        );
    }

    #[test]
    fn a_lane_file_with_an_unknown_schema_names_no_root() {
        let home = tempfile::tempdir().expect("a temporary home");
        let lane_file = home.path().join("omega-exo-lane.json");
        fs::write(
            &lane_file,
            serde_json::json!({ "schema": "something.else.v9", "root": "/tmp" }).to_string(),
        )
        .expect("the fixture lane file is written");

        assert_eq!(
            root_named_by_lane_file(&lane_file),
            None,
            "a root read out of a file whose shape Omega does not recognise is \
             a path with no provenance"
        );
    }

    /// The refusal that started the correction. It named one path, and the
    /// summary drawn from it generalised to the whole machine.
    #[test]
    fn the_absent_root_refusal_names_every_place_it_looked() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_checkout(home.path(), "work/exo", EXO_PIN.upstream);
        write_binary(&checkout, "target/release/exo");
        let cwd = home.path().join("cwd");
        fs::create_dir_all(&cwd).expect("the fixture working directory is created");

        let refusal = derive_lane(
            &ExoLaneOverrides {
                working_directory: Some(cwd.clone()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect_err("neither candidate exists");

        let ExoLaneUnderivable::NoStateRoot { searched } = &refusal else {
            panic!("{refusal:?}");
        };
        assert_eq!(
            searched,
            &[cwd.join(".exo"), checkout.join(".exo")],
            "a refusal that names one place gets read as a statement about \
             every place"
        );
    }

    /// The candidate the correction added is inert in the launch a new person
    /// makes, unless the launcher's directory is rejected.
    ///
    /// macOS starts a bundled application with a working directory of `/`, so a
    /// raw read makes the second candidate `/.exo` on every Finder and Dock
    /// launch. Asserting the *policy* rather than the outcome, because `/.exo`
    /// does not exist on this machine and a test that only checked the derived
    /// lane would pass for that reason instead of this one.
    #[test]
    fn a_launcher_directory_is_not_a_working_directory_candidate() {
        let home = tempfile::tempdir().expect("a temporary home");

        for launcher in ["/", "/Applications/Omega.app/Contents/MacOS", "/usr/bin"] {
            assert_eq!(
                chosen_working_directory(Some(PathBuf::from(launcher)), Some(home.path())),
                None,
                "{launcher} is what a launcher hands over, not somewhere a \
                 person chose to be, and a root derived from it is a lane \
                 pointed at a directory nobody named"
            );
        }
        assert_eq!(
            chosen_working_directory(Some(home.path().to_path_buf()), Some(home.path())),
            None,
            "the home directory itself is rejected by OMEGA-DELTA-0054 and \
             must be rejected here by the same rule, not by a second one"
        );
    }

    #[test]
    fn a_directory_a_person_is_in_is_still_a_working_directory_candidate() {
        let home = tempfile::tempdir().expect("a temporary home");
        let chosen = home.path().join("a-directory-somebody-cd-ed-into");
        fs::create_dir_all(&chosen).expect("the fixture directory is created");

        assert_eq!(
            chosen_working_directory(Some(chosen.clone()), Some(home.path())),
            Some(chosen),
            "gating the candidate must not remove it; this is the candidate \
             that found the only working root this has ever run against"
        );
    }

    /// The gate is applied to the candidate, not to the whole derivation.
    #[test]
    fn a_launcher_directory_does_not_stop_the_checkout_root_from_answering() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());

        let lane = derive_lane(
            &ExoLaneOverrides {
                working_directory: chosen_working_directory(
                    Some(PathBuf::from("/")),
                    Some(home.path()),
                ),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect("the checkout's own root still answers on a Finder launch");

        assert_eq!(lane.root, checkout.join(STATE_ROOT_DIRECTORY));
    }

    #[test]
    fn a_named_state_root_is_used_instead_of_the_checkout_default() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        let elsewhere = home.path().join("state");
        write_record(&agents(&elsewhere).join("id"), "named-root-agent");
        write_record(
            &agents(&elsewhere)
                .join("id")
                .join("conversations")
                .join("id"),
            "named-root-conversation",
        );

        let lane = derive_lane(
            &ExoLaneOverrides {
                root: Some(elsewhere.clone()),
                ..ExoLaneOverrides::default()
            },
            home.path(),
        )
        .expect("a named root is used");

        assert_eq!(lane.root, elsewhere);
        assert_eq!(lane.checkout, checkout, "the checkout is still derived");
        assert_eq!(lane.agent, "named-root-agent");
    }

    /// The lane's own field documentation says the checkout is "the checkout
    /// the binary was built from". A binary found anywhere else carries no
    /// evidence of that, which is why `PATH` is not searched.
    #[test]
    fn the_binary_comes_from_inside_the_checkout() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());

        let lane = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect("this machine has an install");

        assert!(
            lane.binary.starts_with(&checkout),
            "the binary must be the one built in the checkout the pin check \
             reads: {}",
            lane.binary.display()
        );
    }

    #[test]
    fn a_release_binary_is_preferred_over_a_debug_one() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        write_binary(&checkout, "target/debug/exo");

        assert_eq!(
            built_binary(&checkout).expect("both are built"),
            checkout.join("target/release/exo")
        );
    }

    #[test]
    fn a_record_that_is_not_json_is_not_an_agent() {
        let home = tempfile::tempdir().expect("a temporary home");
        let checkout = write_machine(home.path());
        fs::write(
            agents(&checkout.join(STATE_ROOT_DIRECTORY))
                .join("agent-id")
                .join("record.json"),
            "half a file",
        )
        .expect("the fixture record is truncated");

        let refusal = derive_lane(&ExoLaneOverrides::default(), home.path())
            .expect_err("a half-read record is not an agent");

        assert!(matches!(refusal, ExoLaneUnderivable::NoAgent { .. }), "{refusal:?}");
    }
}
