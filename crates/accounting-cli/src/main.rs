//! `pacc` — headless CLI for the personal accounting app.
//!
//! This is not throwaway scaffolding. It is the enforcement mechanism for the
//! core/UI boundary: if `pacc` can do everything the GUI can, then the core is
//! genuinely UI-agnostic and a second frontend is a rendering exercise rather
//! than a re-implementation. It also stays useful forever for backfills,
//! debugging, and integrity checks.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pacc", version, about = "Personal accounting — headless CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print version and build information.
    Version,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Version => {
            println!("pacc {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}
