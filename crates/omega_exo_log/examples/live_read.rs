//! Read a live `exo serve` and print what came back.
//!
//! omega#104. Every other check in this crate runs against a scripted socket,
//! which proves the client agrees with a transcription of the protocol. This
//! proves it agrees with the protocol itself.
//!
//! ```text
//! EXO_EXOHARNESS_URL=127.0.0.1:4766 \
//!   cargo run -p omega_exo_log --example live_read -- <agent-id> <conversation-id>
//! ```
//!
//! Exits non-zero on any refusal, so an unattended run cannot mistake "the
//! server said no" for "there was nothing to read".

use omega_exo_log::{ExoEventWindow, ExoId, ExoQuery, ExoReadClient};

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(agent), Some(conversation)) = (args.next(), args.next()) else {
        eprintln!("usage: live_read <agent-id> <conversation-id>");
        return std::process::ExitCode::FAILURE;
    };
    let (Ok(agent), Ok(conversation)) = (ExoId::parse(&agent), ExoId::parse(&conversation)) else {
        eprintln!("both arguments must be Exo ids");
        return std::process::ExitCode::FAILURE;
    };

    let client = match ExoReadClient::open_default() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("could not open: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let query = ExoQuery::ConversationEvents {
        agent,
        conversation,
        window: ExoEventWindow::default(),
    };
    match client.events(&query) {
        Ok(page) => {
            println!("events read: {}", page.events.len());
            for event in page.events.iter().take(5) {
                println!("  {}  {}", event.id, event.tag());
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("read failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
