//! What detection finds on *this* machine, right now.
//!
//! omega#100. The unit tests run against a fabricated `PATH`, which is what
//! makes them honest — they do not depend on what happens to be installed. This
//! is the other half: it prints what the real machine yields, so a claim about
//! detection can be checked without launching Omega and looking at a window.
//!
//! `cargo run -p omega_agent_detect --example detect`
//!
//! `OMEGA-DELTA-0092` added the Exo half. Exo is not on `PATH` and cannot be —
//! it has no release artifact — so it is reported separately, as either the
//! five fields a lane needs or the one field that is missing and where it was
//! looked for.
//!
//! Exits non-zero when Codex is absent, so an unattended run that is about to
//! assert "the first message routed to Codex" can check its own precondition
//! first. A run that asserts routing on a machine without Codex is testing
//! nothing, and should say so before it starts rather than fail confusingly
//! later.

fn main() -> std::process::ExitCode {
    let detected = omega_agent_detect::detect_from_env();

    if detected.is_empty() {
        println!("no coding agents found on PATH");
    }
    for agent in &detected {
        println!(
            "{:<20} {:<16} {}",
            agent.id,
            agent.name,
            agent.binary.display()
        );
    }

    match omega_agent_detect::exo::derive_lane_from_env() {
        Ok(lane) => {
            println!("\nexo lane:");
            println!("  binary       {}", lane.binary.display());
            println!("  checkout     {}", lane.checkout.display());
            println!("  root         {}", lane.root.display());
            println!("  agent        {}", lane.agent);
            println!("  conversation {}", lane.conversation);
        }
        // Not an error here. A machine with no Exo is the ordinary case, and
        // the point of printing this is that the sentence names the field that
        // is missing rather than saying "not found".
        Err(underivable) => println!("\nexo lane: none — {underivable}"),
    }

    match omega_agent_detect::preferred(&detected) {
        Some(agent) => {
            println!("\npreferred: {} at {}", agent.id, agent.binary.display());
            std::process::ExitCode::SUCCESS
        }
        None => {
            println!("\npreferred: none — Codex is not on PATH");
            std::process::ExitCode::FAILURE
        }
    }
}
