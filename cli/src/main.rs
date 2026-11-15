//! `swiftwin` – build, run and test SwiftUI iOS apps on a real iPhone from Windows.

mod commands;

use clap::Parser;

fn main() {
    let cli = commands::Cli::parse();
    commands::warn_if_sandboxed(&cli);
    if let Err(e) = commands::run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
