//! Run one episode against a live `exo serve`. omega#103, `OMEGA-DELTA-0120`.
//!
//! Every check in `omega_exo_episode` runs against a value this crate wrote
//! down, which proves the law agrees with a *transcription* of Exo. omega#104
//! did the same thing to the read client and the live server contradicted it
//! twice. This is the episode half of that comparison, and it found a third
//! contradiction: see [`PageBound`].
//!
//! ```text
//! script/exo-episode-live <root> <agent-id> <conversation-id>
//! ```
//!
//! or by hand:
//!
//! ```text
//! EXO_EXOHARNESS_URL=127.0.0.1:4766 \
//!   cargo run -p omega_exo_episode --example live_episode -- \
//!   /tmp/exo-serve-copy/.exo <agent-id> <conversation-id> <run-tag>
//! ```
//!
//! **Point this at a copy of an Exo root, never a live one.** It forks, and a
//! fork is a write; `.exo` is single-writer storage and the second writer is
//! how a fork becomes a copy of a history that never existed. The run tag is a
//! required argument rather than a clock reading, because slugs must be unique
//! per run and this file has no clock — the same reason it has no filesystem.
//!
//! # What it proves, and what it cannot
//!
//! It walks [`FALSIFICATION_LOOP`] in order and records each [`Step`] it took,
//! then asserts the record equals the constant. The three omega#103 acceptance
//! items map onto it:
//!
//! * *Two forks from one event start identical* — two forks are taken at one
//!   event and [`EpisodeState::diff`] compares them. The raw reads are also
//!   compared and **must differ**, because a fork re-mints ids: a run where
//!   both comparisons agreed would mean the identity strip was doing nothing.
//! * *A check can be falsified without touching the working tree* — a named
//!   check runs against the compared state, and this file cannot reach a file.
//!   It has no `std::fs`, no `std::process`, and no path type, exactly like the
//!   crate it exercises.
//! * *A discarded fork leaves nothing behind* — the protocol half is here: the
//!   source conversation's digest and `latest_event_id` are re-read at the end
//!   and must be what they were. The filesystem and process half is
//!   `script/exo-episode-live`, because seeing that nothing was written
//!   requires looking at the disk, and this file will not look at a disk.
//!
//! # The mutation, honestly
//!
//! omega#103's loop applies a mutation to the candidate fork. At this pin Omega
//! cannot: `conversation_add_events` is in the refused write family, and a real
//! turn needs an executor and a model. So the mutation this run applies is **the
//! fork point** — the candidate is taken one event later than the control, which
//! is omega#103's own second falsifier ("fork after the mutation rather than
//! before: the sibling must carry the mutation") stated positively. The event
//! that arrives with it is a real event out of the real log, the probe measures
//! it as present, the sibling measures it as absent, and the named check that
//! passed before it fails after it. What is *not* proven here is a mutation
//! authored by Omega, and the run says so in its own output rather than in a
//! footnote.

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::time::Duration;

use omega_exo_episode::{
    AgentId, CheckOutcome, ConversationId, Divergence, EpisodeRequest, EpisodeSession,
    EpisodeShape, EpisodeState, EventId, ExoRoots, FALSIFICATION_LOOP, ForkSlug,
    ForkedConversation, PageBound, ProbeOutcome, SandboxScopeKind, SnapshotEvidence, Step, Verdict,
    admit_filesystem_reset, verdict,
};
use omega_exo_lane::LoopbackEndpoint;

/// The check this run falsifies, by name.
///
/// Named rather than inlined because a verdict about "the check" is worth
/// nothing if nobody can say which check.
const NAMED_CHECK: &str = "every_started_turn_is_ended";

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => {
            println!("EPISODE OK");
            std::process::ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("EPISODE FAILED: {failure}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let (Some(root), Some(agent), Some(conversation), Some(tag)) = (
        arguments.next(),
        arguments.next(),
        arguments.next(),
        arguments.next(),
    ) else {
        return Err("usage: live_episode <exo-root> <agent-id> <conversation-id> <run-tag>".into());
    };
    let agent = AgentId::parse(&agent).map_err(|error| format!("agent id: {error}"))?;
    let conversation = ConversationId::parse(&conversation)
        .map_err(|error| format!("conversation id: {error}"))?;
    let tag = ForkSlug::parse(&tag).map_err(|error| format!("run tag: {error}"))?;

    let address = std::env::var("EXO_EXOHARNESS_URL")
        .unwrap_or_else(|_| omega_exo_lane::EXO_SERVE_DEFAULT_BIND.to_owned());
    let endpoint = LoopbackEndpoint::parse(&address)
        .map_err(|reason| format!("Omega will not drive an episode there: {reason}"))?;

    // One writer per root, and the claim is held for the whole run.
    let mut roots = ExoRoots::new();
    let claim = roots
        .claim(&root)
        .map_err(|reason| format!("that root is already claimed: {reason}"))?;
    let mut session = EpisodeSession::open(endpoint, claim);
    println!("root       {root}");
    println!("endpoint   {}", session.request_url());
    println!("check      {NAMED_CHECK}");

    let mut taken: Vec<Step> = Vec::new();

    // The source. Read before anything is forked, so the end of the run has
    // something to compare against.
    let source_record = show(&mut session, &agent, &conversation)?;
    let source_latest = latest_event_id(&source_record)
        .ok_or_else(|| "the source conversation has no events to fork".to_owned())?;
    let (source_before, source_raw) = read_episode(&mut session, &agent, &conversation)?;
    println!(
        "source     {} events, digest {}, latest {source_latest}",
        source_before.len(),
        short(&source_before.digest())
    );

    // The fork point, and the event that follows it. Chosen out of the real log
    // rather than passed in, so the run cannot be pointed at a pair that makes
    // it look good.
    let (before_index, before, after) = fork_points(&source_raw)?;
    println!("fork point {before} (event {before_index}), mutation carries {after}");

    // ---- Step::ForkCandidate, Step::ForkControl.
    //
    // Both at the same event, and both before anything is applied to either.
    let candidate = fork(&mut session, &agent, &conversation, &before, &tag, "cand")?;
    taken.push(Step::ForkCandidate);
    let control = fork(&mut session, &agent, &conversation, &before, &tag, "ctrl")?;
    taken.push(Step::ForkControl);
    println!("FORK candidate {}", candidate.conversation().as_str());
    println!("FORK control   {}", control.conversation().as_str());

    // ---- Step::ReadCandidateBaseline, Step::ReadControlBaseline.
    let (candidate_before, candidate_raw) = read_fork(&mut session, &candidate)?;
    taken.push(Step::ReadCandidateBaseline);
    let (control_before, control_raw) = read_fork(&mut session, &control)?;
    taken.push(Step::ReadControlBaseline);

    // ---- Step::CompareStartingStates. omega#103's second acceptance item,
    // decided by comparing rather than by asserting.
    let start = candidate_before.diff(&control_before);
    taken.push(Step::CompareStartingStates);
    println!(
        "start      candidate {} ({} events) against control {} ({} events): {start}",
        short(&candidate_before.digest()),
        candidate_before.len(),
        short(&control_before.digest()),
        control_before.len()
    );
    if start != Divergence::Identical {
        return Err(format!(
            "two forks of one event did not start identical: {start}"
        ));
    }
    if candidate_before.digest() != control_before.digest() {
        return Err("the diff says identical and the digests disagree".into());
    }
    // And the other direction, which is the load-bearing half: a fork re-mints
    // every event id, so the *raw* reads must differ. If they matched, the
    // identity strip would be comparing nothing and this whole check would pass
    // on two readings of one conversation.
    if candidate_raw == control_raw {
        return Err(
            "two forks read byte-identical, so either Exo stopped re-minting event ids \
             or this run compared one conversation with itself"
                .into(),
        );
    }
    println!("start      raw reads differ, as two forks must; identity-stripped states match");

    // ---- Step::ApplyMutationInCandidate.
    //
    // The fork point is the mutation; see the module docs. The candidate after
    // the mutation is a fork of the same source one event later.
    let mutated = fork(&mut session, &agent, &conversation, &after, &tag, "mut")?;
    taken.push(Step::ApplyMutationInCandidate);
    println!("FORK mutated   {}", mutated.conversation().as_str());

    // ---- Step::ReadCandidateAfterMutation, Step::ReadControlAfterMutation.
    let (candidate_after, _) = read_fork(&mut session, &mutated)?;
    taken.push(Step::ReadCandidateAfterMutation);
    let (control_after, _) = read_fork(&mut session, &control)?;
    taken.push(Step::ReadControlAfterMutation);

    // ---- Step::ProbeMutationApplied. Before the check, always. A check run
    // against a mutation that never applied answers a question nobody asked,
    // and reports green while doing it.
    let moved = candidate_after.diff(&candidate_before);
    let probe = if moved == Divergence::Identical {
        ProbeOutcome::MutationAbsent
    } else {
        ProbeOutcome::MutationPresent
    };
    taken.push(Step::ProbeMutationApplied);
    println!("probe      candidate against its baseline: {moved} -> {probe:?}");

    // ---- Step::CompareSiblingUnmutated.
    let sibling = control_after.diff(&control_before);
    taken.push(Step::CompareSiblingUnmutated);
    println!("sibling    control against its baseline: {sibling}");
    if sibling != Divergence::Identical {
        return Err(format!(
            "the sibling moved while the candidate was mutated, so the fork isolates nothing: \
             {sibling}"
        ));
    }

    // ---- Step::RunNamedCheck. Over the compared state, not over a second
    // reading of it.
    let on_control = every_started_turn_is_ended(&control_after);
    let on_candidate = every_started_turn_is_ended(&candidate_after);
    taken.push(Step::RunNamedCheck);
    println!("check      {NAMED_CHECK} on control: {on_control:?}");
    println!("check      {NAMED_CHECK} on candidate: {on_candidate:?}");
    if on_control != CheckOutcome::Passed {
        return Err(format!(
            "{NAMED_CHECK} already fails on the unmutated control, so its failure on the \
             candidate would say nothing about the mutation"
        ));
    }

    // ---- Step::ReadVerdict.
    let outcome = verdict(probe, on_candidate);
    taken.push(Step::ReadVerdict);
    println!("verdict    {outcome:?}: {outcome}");
    if outcome != Verdict::Falsified {
        return Err(format!("the loop did not falsify the check: {outcome}"));
    }

    if taken != FALSIFICATION_LOOP {
        return Err(format!(
            "this run took {taken:?}, which is not the declared loop {FALSIFICATION_LOOP:?}"
        ));
    }
    println!(
        "loop       {} steps, in FALSIFICATION_LOOP order",
        taken.len()
    );

    // The source, again. Nothing in this run may have reached it.
    let (source_after, _) = read_episode(&mut session, &agent, &conversation)?;
    if source_after.digest() != source_before.digest() {
        return Err(format!(
            "the source conversation changed under the run: {} then {}",
            source_before.digest(),
            source_after.digest()
        ));
    }
    let source_record_after = show(&mut session, &agent, &conversation)?;
    let latest_after = latest_event_id(&source_record_after).unwrap_or_default();
    if latest_after != source_latest {
        return Err(format!(
            "the source's latest event moved from {source_latest} to {latest_after}"
        ));
    }
    println!(
        "source     unchanged: {} events, digest {}, latest {latest_after}",
        source_after.len(),
        short(&source_after.digest())
    );

    // The filesystem half, decided rather than attempted. The evidence is read
    // off the durable log this run already has in hand: if no event in the
    // source names a snapshot, there is nothing to restore, and Exo would
    // answer a `start_sandbox` with a sentence about a missing manifest that
    // reads like the fork bug and is not.
    let evidence = if names_a_snapshot(&source_before) {
        SnapshotEvidence::Observed
    } else {
        SnapshotEvidence::NoneObserved
    };
    println!("snapshots  evidence in the source log: {evidence:?}");
    for (scope, shape) in [
        (SandboxScopeKind::Agent, EpisodeShape::SingleEpisode),
        (SandboxScopeKind::Agent, EpisodeShape::Siblings),
        (SandboxScopeKind::Conversation, EpisodeShape::SingleEpisode),
        (SandboxScopeKind::Turn, EpisodeShape::SingleEpisode),
    ] {
        match admit_filesystem_reset(scope, shape, evidence) {
            Ok(_) => println!("reset      {scope:?}/{shape:?}: admitted"),
            Err(refusal) => println!("reset      {scope:?}/{shape:?}: refused — {refusal}"),
        }
    }

    session.close(&mut roots);
    Ok(())
}

/// `every_started_turn_is_ended`, over the state two episodes were compared by.
///
/// Exo appends `turn_started` when a turn opens and `turn_ended` when it
/// closes, so an episode cut between the two carries a turn that never
/// finished. That is the property this run breaks by moving the fork point one
/// event, and it is a property of the durable log rather than of a rendering.
fn every_started_turn_is_ended(state: &EpisodeState) -> CheckOutcome {
    let mut open = 0i64;
    for event in state.events() {
        match event
            .get("data")
            .and_then(|data| data.get("type"))
            .and_then(serde_json::Value::as_str)
        {
            Some("turn_started") => open += 1,
            Some("turn_ended") => open -= 1,
            _ => {}
        }
    }
    if open == 0 {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed
    }
}

/// Whether anything in the durable log names a snapshot for a sandbox.
fn names_a_snapshot(state: &EpisodeState) -> bool {
    state.events().iter().any(|event| {
        let Some(data) = event.get("data") else {
            return false;
        };
        let tagged_snapshot = data
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|tag| tag == "sandbox_snapshotted");
        let carries_snapshot_id = data
            .get("snapshot_id")
            .is_some_and(|value| !value.is_null());
        tagged_snapshot || carries_snapshot_id
    })
}

/// The fork point and the event after it.
///
/// The pair is the last `turn_started` in the log and the event before it: a
/// fork at the earlier one has every turn closed, and a fork at the later one
/// has exactly one turn open. Both are real events out of the real log.
fn fork_points(events: &[serde_json::Value]) -> Result<(usize, String, String), String> {
    let index = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event
                .get("data")
                .and_then(|data| data.get("type"))
                .and_then(serde_json::Value::as_str)
                == Some("turn_started")
        })
        .map(|(index, _)| index)
        .next_back()
        .ok_or_else(|| "no turn ever started in this conversation".to_owned())?;
    if index == 0 {
        return Err("the conversation opens with a turn, so there is no event before it".into());
    }
    let id = |at: usize| -> Result<String, String> {
        events
            .get(at)
            .and_then(|event| event.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("the event at {at} has no id"))
    };
    Ok((index - 1, id(index - 1)?, id(index)?))
}

fn latest_event_id(record: &serde_json::Value) -> Option<String> {
    record
        .get("response")?
        .get("conversation")?
        .get("record")?
        .get("latest_event_id")?
        .as_str()
        .map(str::to_owned)
}

fn short(digest: &str) -> String {
    digest.chars().take(12).collect()
}

fn show(
    session: &mut EpisodeSession,
    agent: &AgentId,
    conversation: &ConversationId,
) -> Result<serde_json::Value, String> {
    let request = EpisodeRequest::ShowConversation {
        agent: agent.clone(),
        conversation: conversation.clone(),
    };
    let (id, body) = session.prepare(&request);
    let reply = post(session, &body)?;
    if reply.get("id").and_then(serde_json::Value::as_u64) != Some(id) {
        return Err(format!(
            "a conversation read answered request {id} with somebody else's"
        ));
    }
    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(format!(
            "Exo refused the conversation read: {}",
            reply.get("error").unwrap_or(&serde_json::Value::Null)
        ));
    }
    Ok(reply)
}

fn fork(
    session: &mut EpisodeSession,
    agent: &AgentId,
    conversation: &ConversationId,
    up_to_inclusive: &str,
    tag: &ForkSlug,
    role: &str,
) -> Result<ForkedConversation, String> {
    let slug = ForkSlug::parse(&format!("ep-{}-{role}", tag.as_str()))
        .map_err(|error| format!("fork slug: {error}"))?;
    let request = EpisodeRequest::ForkAtEvent {
        agent: agent.clone(),
        conversation: conversation.clone(),
        up_to_inclusive: EventId::parse(up_to_inclusive)
            .map_err(|error| format!("fork point: {error}"))?,
        slug: Some(slug),
    };
    let (id, body) = session.prepare(&request);
    let reply = post(session, &body)?;
    ForkedConversation::read_fork_response(id, &reply).map_err(|error| error.to_string())
}

fn read_fork(
    session: &mut EpisodeSession,
    fork: &ForkedConversation,
) -> Result<(EpisodeState, Vec<serde_json::Value>), String> {
    read_episode(session, fork.agent(), fork.conversation())
}

/// Read a whole conversation, and prove the page was whole.
///
/// `OMEGA-DELTA-0120`. Exo hands back the last event's id as the cursor for
/// every non-empty page, so the cursor cannot be read as "there is more". The
/// proof that this page is the episode is a second read resuming from that
/// cursor, which must come back empty. Two round trips per read, and the second
/// is the one that makes the first a fact.
fn read_episode(
    session: &mut EpisodeSession,
    agent: &AgentId,
    conversation: &ConversationId,
) -> Result<(EpisodeState, Vec<serde_json::Value>), String> {
    let request = EpisodeRequest::ReadEvents {
        agent: agent.clone(),
        conversation: conversation.clone(),
        limit: None,
        after: None,
    };
    let (id, body) = session.prepare(&request);
    let reply = post(session, &body)?;
    let state = EpisodeState::read_events_response(id, &reply, PageBound::WholeLog)
        .map_err(|error| error.to_string())?;
    let raw = reply
        .get("response")
        .and_then(|response| response.get("result"))
        .and_then(|result| result.get("events"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    if let Some(cursor) = EpisodeState::read_cursor(&reply) {
        let resume = EpisodeRequest::ReadEvents {
            agent: agent.clone(),
            conversation: conversation.clone(),
            limit: None,
            after: Some(EventId::parse(&cursor).map_err(|error| format!("cursor: {error}"))?),
        };
        let (resume_id, resume_body) = session.prepare(&resume);
        let resume_reply = post(session, &resume_body)?;
        let tail =
            EpisodeState::read_events_response(resume_id, &resume_reply, PageBound::WholeLog)
                .map_err(|error| error.to_string())?;
        if !tail.is_empty() {
            return Err(format!(
                "the read stopped short: {} more events after {cursor}, so the state \
                 compared would have been a prefix",
                tail.len()
            ));
        }
    } else if !state.is_empty() {
        return Err("a non-empty page came back with no cursor, which Exo does not do".into());
    }
    Ok((state, raw))
}

/// One request, one connection, on this machine.
///
/// `LoopbackEndpoint` already refused anything that is not this machine, and
/// the resolved address is checked again for the reason `omega_exo_log` checks
/// it twice: `localhost` is a name, and a name resolves through `/etc/hosts`.
/// No `Authorization` header, ever — `exo serve` does not read one, and sending
/// it would assert an authentication that does not exist.
fn post(session: &EpisodeSession, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    const TIMEOUT: Duration = Duration::from_secs(30);
    let port = session.endpoint().port().unwrap_or(4766);
    let host = session.endpoint().host();
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let address = authority
        .to_socket_addrs()
        .map_err(|error| format!("{authority} did not resolve: {error}"))?
        .next()
        .ok_or_else(|| format!("{authority} resolved to nothing"))?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "{authority} resolved to {address}, which is not this machine"
        ));
    }

    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)
        .map_err(|error| format!("could not reach Exo at {address}: {error}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|error| format!("socket: {error}"))?;
    let rendered = body.to_string();
    let request = format!(
        "POST {} HTTP/1.1\r\n\
         host: {address}\r\n\
         content-type: application/json\r\n\
         accept: application/json\r\n\
         content-length: {}\r\n\
         connection: close\r\n\r\n{rendered}",
        omega_exo_episode::EXO_SERVE_REQUEST_PATH,
        rendered.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|error| format!("writing to Exo: {error}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|error| format!("reading from Exo: {error}"))?;
    let text = String::from_utf8_lossy(&raw);
    let (head, payload) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "Exo's reply had no body".to_owned())?;
    let status = head.split_whitespace().nth(1).unwrap_or("?");
    if !status.starts_with('2') {
        return Err(format!("Exo answered HTTP {status}"));
    }
    serde_json::from_str(payload).map_err(|error| format!("Exo's reply was not JSON: {error}"))
}
