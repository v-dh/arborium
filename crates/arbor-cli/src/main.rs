mod cli;
mod commands;

use {arbor_daemon_client::DaemonClient, clap::Parser, std::process::ExitCode};

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let client = DaemonClient::from_env();

    let result = commands::run(&cli, &client);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                let json = serde_json::json!({ "error": error.to_string() });
                match serde_json::to_string_pretty(&json) {
                    Ok(s) => println!("{s}"),
                    Err(e) => eprintln!("error: failed to serialize JSON: {e}"),
                }
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::FAILURE
        },
    }
}
