//! SCV process entry: CLI, stderr diagnostics, stdio ACP serve, exit status.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Diagnostics strictly on stderr — never stdout (protocol-only).
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("scv=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .init();

    let args: Vec<String> = std::env::args().collect();
    let root_args = match scv::parse_cli_args(args) {
        Ok(roots) => roots,
        Err(message) => {
            // Help text is not an error.
            if message.starts_with("SCV v0.1") {
                eprintln!("{message}");
                return ExitCode::SUCCESS;
            }
            eprintln!("scv: {message}");
            return ExitCode::from(2);
        }
    };

    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("scv: failed to resolve current directory: {error}");
            return ExitCode::from(2);
        }
    };

    let roots = match scv::ReadRoots::new(root_args, &current_dir) {
        Ok(roots) => roots,
        Err(message) => {
            eprintln!("scv: {message}");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("scv: failed to start async runtime: {error}");
            return ExitCode::from(1);
        }
    };

    match runtime.block_on(scv::serve(roots)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("scv: {error}");
            ExitCode::from(1)
        }
    }
}
