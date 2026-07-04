mod capture;
mod cli;
mod clip;
mod config;
mod daemon;
mod doctor;
mod freeze;
mod grab;
mod menu;
mod model;
mod notify;
mod picker;
mod process;
mod prompt;
mod restore;
mod search;
mod session;
mod status;
mod store;
mod switcher;
mod tmux;
mod url;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Cmd};

fn main() {
    if let Err(e) = run() {
        // Via notify: a popup's stderr vanishes with the popup, so inside
        // tmux the error is also surfaced on the status line.
        notify::error(&format!("{e:#}"));
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
        Cmd::Pick => switcher::run(),
        Cmd::Freeze { name, script } => freeze::freeze(name.as_deref(), script),
        Cmd::Up { name } => freeze::up(&name),
        Cmd::Switch => switcher::run(),
        Cmd::Url { pane, source, client } => {
            url::run(pane.as_deref(), source.as_deref(), client.as_deref())
        }
        Cmd::Grab { pane, source, client } => {
            grab::run(pane.as_deref(), source.as_deref(), client.as_deref())
        }
        Cmd::Search { pane, source, client } => {
            search::run(pane.as_deref(), source.as_deref(), client.as_deref())
        }
        Cmd::Clip { primary } => clip::run(primary),
        // `--session` is still accepted (binding compat) but ignored: nothing
        // in the menu flow ever consumed it (dead plumbing removed).
        Cmd::Menu { run, client, session: _ } => menu::run(run, client.as_deref()),
        Cmd::Session { action } => session::run(action),
        Cmd::Status => status::print(),
        Cmd::Doctor => doctor::run(),
        Cmd::Daemon => daemon::run(),
        Cmd::Hook { event } => daemon::hook(&event),
        Cmd::Autostart => restore::autostart(),
    }
}
