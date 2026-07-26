//! What detection finds on *this* machine, right now.
//!
//! omega#100. The unit tests run against a fabricated `PATH`, which is what
//! makes them honest — they do not depend on what happens to be installed. This
//! is the other half: it prints what the real machine yields, so a claim about
//! detection can be checked without launching Omega and looking at a window.
//!
//! `cargo run -p omega_agent_detect --example detect`
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
