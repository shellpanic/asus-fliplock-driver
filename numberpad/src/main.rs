mod control;
mod discovery;
mod engine;
#[cfg(feature = "test-fixtures")]
mod fixtures;
mod keymap;
mod linux;
mod manage;
mod runtime;
mod session;
mod sleep;
mod util;
use clap::{Parser, Subcommand, ValueEnum};
#[derive(Parser)]
#[command(version, about = "ASUS NumberPad native session driver")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    BuildInfo,
    Status,
    CheckSession,
    Keymap,
    #[cfg(feature = "test-fixtures")]
    #[command(name = "_fixture-stop", hide = true)]
    FixtureStop {
        role: String,
    },
    #[cfg(feature = "test-fixtures")]
    #[command(name = "_fixture", hide = true)]
    Fixture {
        role: String,
        #[arg(long)]
        fail_ready: bool,
        #[arg(long)]
        fail_cleanup: bool,
    },
    Probe {
        #[arg(long)]
        json: bool,
    },
    Install {
        mode: Mode,
        #[arg(long)]
        yes: bool,
    },
    Setup {
        mode: Mode,
        #[arg(long)]
        yes: bool,
    },
    Disable {
        mode: Mode,
        #[arg(long)]
        yes: bool,
    },
    Run,
    Session,
    Recover,
    Gesture {
        gesture: Gesture,
        #[arg(value_enum, default_value = "apply")]
        mode: Mode,
    },
    #[command(name = "_permissions", hide = true)]
    Permissions {
        user: String,
        action: String,
        #[arg(long)]
        expected: String,
        #[arg(long)]
        modules: bool,
        #[arg(long)]
        no_modules: bool,
    },
    #[command(name = "_match", hide = true)]
    Match {
        path: String,
    },
    #[command(name = "_refresh", hide = true)]
    Refresh {
        path: String,
    },
}
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Plan,
    Apply,
}
#[derive(Clone, Copy, ValueEnum)]
enum Gesture {
    DoubleTap,
    Hold,
}
fn dispatch() -> anyhow::Result<()> {
    match Cli::parse().command {
        Action::BuildInfo => println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"test_fixtures":cfg!(feature="test-fixtures")})
        ),
        Action::Keymap => {
            let map = keymap::resolve(&keymap::source()?)?;
            println!("{}", serde_json::to_string(&map)?);
        }
        Action::CheckSession => {
            if !session::active()? {
                anyhow::bail!("Inactive, locked, remote or mismatched login");
            }
            let map = keymap::resolve(&keymap::source()?)?;
            println!(
                "{} session/keymap verified: {} printed symbols",
                session::display_kind()?,
                map.len()
            );
        }
        #[cfg(feature = "test-fixtures")]
        Action::Fixture {
            role,
            fail_ready,
            fail_cleanup,
        } => fixtures::owner(&role, !fail_ready, fail_cleanup)?,
        #[cfg(feature = "test-fixtures")]
        Action::FixtureStop { role } => control::stop(&role)?,
        Action::Status => {
            let device = discovery::probe()?;
            let (available, error) = match control::ready("runtime") {
                Ok(value) => (Some(value), None),
                Err(error) => (None, Some(error.to_string())),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"hardware":device,"available":available,"process_error":error,"activation":manage::settings()?.activation})
                )?
            );
        }
        Action::Probe { .. } => println!("{}", serde_json::to_string_pretty(&discovery::probe()?)?),
        Action::Install { mode, yes } | Action::Setup { mode, yes } => {
            manage::install(matches!(mode, Mode::Apply), yes)?
        }
        Action::Disable { mode, yes } => manage::disable(matches!(mode, Mode::Apply), yes)?,
        Action::Run => runtime::run(manage::settings()?)?,
        Action::Session => manage::supervise()?,
        Action::Recover => runtime::recover(false)?,
        Action::Gesture { gesture, mode } => manage::gesture(
            match gesture {
                Gesture::DoubleTap => engine::Activation::DoubleTap,
                Gesture::Hold => engine::Activation::Hold,
            },
            matches!(mode, Mode::Apply),
        )?,
        Action::Permissions {
            user,
            action,
            expected,
            modules,
            no_modules,
        } => {
            if modules == no_modules || !["enable", "disable"].contains(&action.as_str()) {
                anyhow::bail!("Invalid fixed permission operation");
            }
            manage::permissions(&user, action == "enable", modules, &expected)?;
        }
        Action::Match { path } => {
            if let Err(error) = manage::matching(&path, false) {
                eprintln!("No exact device match: {error}");
            }
        }
        Action::Refresh { path } => manage::matching(&path, true)?,
    }
    Ok(())
}
fn main() {
    if let Err(error) = dispatch() {
        eprintln!("asus-numberpad: {error:#}");
        std::process::exit(1);
    }
}
