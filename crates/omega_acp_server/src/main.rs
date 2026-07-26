//! The loopback ACP server as a supervised child. `OMEGA-DELTA-0041`,
//! omega#82.
//!
//! `omega-effectd` owns the lifecycle in a shipped build, by calling
//! [`omega_acp_server::start_if_enabled`] from `crates/omega_effectd`. This
//! binary exists so the same code path can be **driven by a real external ACP
//! host** without a windowing system: an external host attaches through a
//! stdio-to-TCP bridge, and what it talks to is the library above, not a
//! demonstration copy of it.
//!
//! It is off by the same flag and binds through the same
//! [`omega_acp_server::LoopbackHost`], so running it proves the shipped
//! behaviour rather than a parallel one. With the flag unset it prints why it
//! is not listening and exits, which is the default-off property observable
//! from a shell.
//!
//! # Attaching an external ACP host
//!
//! ACP is spoken over a subprocess's stdio, so an external host reaches a
//! socket through a one-line bridge. This is exactly how stock Zed 1.12.0 was
//! driven against it on 2026-07-26:
//!
//! ```text
//! OMEGA_ACP_SERVER=1 OMEGA_ACP_SERVER_PORT=8282 omega-acp-server
//! ```
//!
//! then, in the external host's settings:
//!
//! ```json
//! "agent_servers": {
//!   "omega-served": {
//!     "command": "/bin/sh",
//!     "args": ["-c", "exec /usr/bin/nc 127.0.0.1 8282"]
//!   }
//! }
//! ```
//!
//! Opening a thread on that agent initialises, creates a session, and shows the
//! disclosure the served turn renders. The headless equivalent —
//! the **upstream** ACP SDK client over the same loopback socket — is
//! `the_upstream_acp_client_reads_the_disclosure_off_a_real_socket` in the
//! library, so the exit is reproducible without a windowing system.
//!
//! # The rendered exit, and why the stop reason is `end_turn`
//!
//! Photographed on 2026-07-26 against stock Zed 1.12.0:
//! `evidence/2026-07-26-zed-1.12.0-served-turn-end_turn.png`. The prompt
//! *"Start a Full Auto run on this project and pin the executor."* is kept in
//! the thread and answered with the executor disclosure, the session origin
//! (`loopback_acp · zed 1.12.0+stable… · unauthenticated`), and the statement
//! that the turn reached no executor.
//!
//! The companion image
//! `evidence/2026-07-26-zed-1.12.0-served-turn-refusal.png` is the same build,
//! the same host and the same prompt with **one** value changed — the served
//! turn answering ACP's `refusal` instead of `end_turn`. Stock Zed implements
//! `refusal` literally: it drops the turn out of the thread and renders a bare
//! "Request Refused" banner whose text guesses at a *content policy* violation
//! that never occurred. The disclosure goes with the turn, so the operator of
//! the external host is told nothing true.
//!
//! That pair is why the stop reason is not a cosmetic choice: a test asserting
//! `stopReason == "refusal"` was green while the only human who could act on
//! the refusal could see none of it. A served turn ends, and says what did not
//! happen.

fn main() {
    match omega_acp_server::start_if_enabled() {
        omega_acp_server::StartOutcome::NotStarted(reason) => {
            println!(
                "omega-acp-server: not listening ({}). Set {}={} to serve Omega \
                 Agent over ACP on loopback.",
                reason.token(),
                omega_acp_server::ENABLE_FLAG,
                omega_acp_server::ENABLE_VALUE,
            );
        }
        omega_acp_server::StartOutcome::Listening(address) => {
            // Printed on stdout so a bridge script can read the port back
            // without parsing a log.
            println!("omega-acp-server: listening on {address}");
            // The listener serves on its own thread; park this one on it.
            loop {
                std::thread::park();
            }
        }
        omega_acp_server::StartOutcome::Failed(error) => {
            eprintln!("omega-acp-server: could not listen: {error}");
            std::process::exit(1);
        }
    }
}
