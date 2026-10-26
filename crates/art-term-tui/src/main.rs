#[cfg(unix)]
mod backend;
#[cfg(unix)]
mod desktop;
#[cfg(unix)]
mod display;
#[cfg(unix)]
mod image_view;
#[cfg(unix)]
mod keybindings;
#[cfg(unix)]
mod layout;
#[cfg(unix)]
mod preview;
#[cfg(unix)]
mod tab;
#[cfg(unix)]
mod terminal;
#[cfg(unix)]
mod tty;
#[cfg(unix)]
mod ui;
#[cfg(unix)]
mod vault;

#[cfg(unix)]
use std::{io::IsTerminal, path::PathBuf, process::ExitCode};

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run { config: Option<PathBuf> },
    Help,
    Version,
}

#[cfg(unix)]
fn command(args: impl IntoIterator<Item = std::ffi::OsString>) -> anyhow::Result<Command> {
    let mut config = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => return Ok(Command::Help),
            Some("--version" | "-V") => return Ok(Command::Version),
            Some("--config") => {
                anyhow::ensure!(config.is_none(), "--config may only be specified once");
                config =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--config requires a path")
                    })?));
            }
            Some(value) if value.starts_with("--config=") => {
                anyhow::ensure!(
                    config.is_none() && value.len() > 9,
                    "--config requires one nonempty path"
                );
                config = Some(PathBuf::from(&value[9..]));
            }
            _ => anyhow::bail!("Unknown argument. Use art-term --help."),
        }
    }
    Ok(Command::Run { config })
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("art-term: {error}");
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::Interrupted)
            {
                ExitCode::from(130)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(unix)]
async fn run() -> anyhow::Result<()> {
    let config = match command(std::env::args_os().skip(1))? {
        Command::Help => {
            println!("art-term {}\n\nRemote Host/Shell terminal client for Ubuntu Server and Desktop.\n\nUsage: art-term [--config PATH]\n       art-term --help\n       art-term --version\n\nRuns directly in your terminal or SSH session. No graphical desktop is required.\nCredentials are encrypted using a private key belonging to your Unix account. No unlock password is required.\n\nInside the app: Ctrl-] then ? opens help; Ctrl-] then q quits.\nCtrl-C and Ctrl-D are sent to the selected remote Shell.\n", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Command::Version => {
            println!("art-term {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Command::Run { config } => config,
    };
    anyhow::ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "A terminal is required. Open an interactive terminal or SSH session."
    );
    anyhow::ensure!(
        std::env::var_os("TERM").is_none_or(|term| term != "dumb"),
        "TERM=dumb is not supported; use a terminal with cursor control."
    );
    let paths = vault::VaultPaths::resolve(config)?;
    let bindings = keybindings::Bindings::load(&paths.keybindings_path())?;
    let display = display::load_preferences(&paths.display_path());
    let startup_signals = tty::StartupSignals::install()?;
    let vault = vault::Vault::open_account(paths).map_err(|_| anyhow::anyhow!(
        "Unable to open account-protected storage. Check private directory/key permissions and concurrent sessions. Existing profiles without an account key require explicit migration; files were preserved."
    ))?;
    let backend = backend::Backend::start(vault)?;
    let result = ui::run(backend.clone(), startup_signals, bindings, display).await;
    backend.shutdown().await;
    result
}

#[cfg(not(unix))]
fn main() {
    eprintln!("art-term terminal client requires Ubuntu or another Unix terminal.");
    std::process::exit(1);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> anyhow::Result<Command> {
        command(args.iter().map(std::ffi::OsString::from))
    }
    #[test]
    fn help_and_version_do_not_require_a_terminal_or_vault() {
        assert_eq!(parse(&["--help"]).unwrap(), Command::Help);
        assert_eq!(parse(&["--version"]).unwrap(), Command::Version);
    }
    #[test]
    fn config_preserves_paths_and_rejects_unknown_password_arguments() {
        assert_eq!(
            parse(&["--config", "my data/connections.toml"]).unwrap(),
            Command::Run {
                config: Some("my data/connections.toml".into())
            }
        );
        assert!(parse(&["--config="]).is_err());
        assert!(parse(&["--config", "one", "--config", "two"]).is_err());
        assert!(parse(&["--password", "secret"]).is_err());
    }
}
