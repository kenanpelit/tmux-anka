mod capture;
mod cli;
mod config;
mod daemon;
mod freeze;
mod model;
mod process;
mod restore;
mod session;
mod status;
mod store;
// Wired into the CLI in the switcher's final task; silence until then.
#[allow(dead_code, unused_imports)]
mod switcher;
mod tmux;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Cmd};

fn main() {
    if let Err(e) = run() {
        eprintln!("anka: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Save { name } => capture::save(name.as_deref()),
        Cmd::Restore { name } => restore::restore(name.as_deref()),
        Cmd::List => store::list_cmd(),
        Cmd::Rm { name } => store::rm_cmd(&name),
        Cmd::Pick => tui::pick(),
        Cmd::Freeze { name, script } => freeze::freeze(name.as_deref(), script),
        Cmd::Up { name } => freeze::up(&name),
        Cmd::Session { action } => session::run(action),
        Cmd::Status => status::print(),
        Cmd::Daemon => daemon::run(),
        Cmd::Hook { event } => daemon::hook(&event),
        Cmd::Autostart => restore::autostart(),
    }
}
