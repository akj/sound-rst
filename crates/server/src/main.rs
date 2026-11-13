//! `soundrts-server`: runs a SoundRTS server until Ctrl+C.

use std::io::IsTerminal;
use std::process::ExitCode;

use soundrts_server::{ServerConfig, UpnpOutcome};
use tracing::{error, info};

const USAGE: &str = "\
Usage: soundrts-server [options]

Options:
  --port <port>         TCP port for players and UDP port for local discovery (default 2501)
  --name <name>         the server's name on the local network (default SoundRTS)
  --max-clients <n>     the most players at once (default 40)
  --max-rooms <n>       the most rooms at once (default 10)
  --upnp                ask the router to forward the port
  --help                show this help
";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let config = match parse(std::env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let server = match soundrts_server::start(config) {
        Ok(server) => server,
        Err(e) => {
            error!(error = %e, "cannot start the server");
            return ExitCode::FAILURE;
        }
    };
    info!("press Ctrl+C to stop");
    match server.upnp() {
        UpnpOutcome::Off => {}
        UpnpOutcome::Mapped { external } => {
            info!(%external, "players on the internet can connect to this address")
        }
        UpnpOutcome::Failed { reason } => {
            info!(%reason, "players on the internet may not reach this server")
        }
    }
    let waited = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|rt| rt.block_on(tokio::signal::ctrl_c()));
    if let Err(e) | Ok(Err(e)) = waited {
        error!(error = %e, "cannot wait for Ctrl+C");
    }
    server.shutdown();
    ExitCode::SUCCESS
}

/// The configuration the arguments ask for, or `None` for help.
fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<ServerConfig>, String> {
    let mut config = ServerConfig::default();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--port" => config.port = number(&arg, value(&arg)?)?,
            "--name" => config.name = value(&arg)?,
            "--max-clients" => config.max_clients = number(&arg, value(&arg)?)?,
            "--max-rooms" => config.max_rooms = number(&arg, value(&arg)?)?,
            "--upnp" => config.upnp = true,
            "--help" | "-h" => return Ok(None),
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    Ok(Some(config))
}

fn number<T: std::str::FromStr>(name: &str, value: String) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{name} needs a number, not {value:?}"))
}
