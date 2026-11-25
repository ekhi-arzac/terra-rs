use serde::Deserialize;
use std::{env, error::Error, fs, net::SocketAddr, num::NonZeroU8, path::PathBuf};
use std::{io::BufRead, num::NonZeroU32};
use terraria_network::{Administration, ConsoleCommand};
use tokio::{net::TcpListener, sync::mpsc};
use tracing::info;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    bind: SocketAddr,
    max_players: NonZeroU8,
    password: String,
    world: Option<PathBuf>,
    autosave_seconds: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: ([127, 0, 0, 1], 7777).into(),
            max_players: NonZeroU8::new(8).unwrap(),
            password: String::new(),
            world: None,
            autosave_seconds: 600,
        }
    }
}

fn console() -> mpsc::Receiver<ConsoleCommand> {
    let (send, recv) = mpsc::channel(8);
    // A blocking stdin read must not keep Tokio running after SIGTERM.
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let command = match line.trim().to_ascii_lowercase().as_str() {
                "save" => ConsoleCommand::Save,
                "playing" => ConsoleCommand::Playing,
                "exit" => ConsoleCommand::Exit,
                "" => continue,
                "help" => {
                    println!(
                        "save     Save world changes now\nplaying  List connected players\nexit     Save and stop the server"
                    );
                    continue;
                }
                _ => {
                    println!("Unknown command. Type help for available commands.");
                    continue;
                }
            };
            if send.blocking_send(command).is_err() {
                break;
            }
        }
    });
    recv
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result, _ = term.recv() => Ok(()) }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt().init();
    let mut args = env::args_os().skip(1);
    let config: Config = match (args.next(), args.next(), args.next()) {
        (None, None, None) => Config::default(),
        (Some(flag), Some(path), None) if flag == "--config" => {
            toml::from_str(&fs::read_to_string(path)?)?
        }
        (Some(flag), None, None) if flag == "--help" => {
            println!("Usage: terraria-server [--config FILE.toml]");
            return Ok(());
        }
        _ => return Err("usage: terraria-server [--config FILE.toml]".into()),
    };
    let world = config
        .world
        .as_ref()
        .map(terraria_world::World::load)
        .transpose()?;
    if let Some(w) = &world {
        let m = w.metadata();
        info!(world = %m.name, width = m.width, height = m.height, "loaded world");
    }
    let listener = TcpListener::bind(config.bind).await?;
    info!(address = %listener.local_addr()?, slots = %config.max_players, "listening for Terraria clients");
    terraria_network::serve_managed(
        listener,
        config.max_players,
        config.password,
        world,
        config.world,
        Administration {
            autosave_seconds: NonZeroU32::new(config.autosave_seconds),
            commands: console(),
        },
        async {
            if let Err(error) = shutdown_signal().await {
                tracing::error!(%error, "shutdown signal handler failed");
            }
        },
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_defaults_and_validation() {
        let c: Config = toml::from_str("password = 'á'").unwrap();
        assert_eq!(c.bind, Config::default().bind);
        assert_eq!(c.password, "á");
        for text in [
            "max_players = 0",
            "max_players = 256",
            "port = 1234",
            "bind = 'bad'",
        ] {
            assert!(toml::from_str::<Config>(text).is_err());
        }
    }
}
