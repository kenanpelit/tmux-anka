use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "anka", version, about = "Freeze and resurrect tmux sessions, exactly.")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Save the current tmux environment to a snapshot
    Save { name: Option<String> },
    /// Restore a snapshot (default: last)
    Restore { name: Option<String> },
    /// List saved snapshots
    List,
    /// Remove a snapshot
    Rm { name: String },
    /// Interactively pick a session to restore
    Pick,
    /// Freeze a snapshot into a re-runnable blueprint
    Freeze {
        name: Option<String>,
        /// Also export a standalone shell script
        #[arg(long)]
        script: bool,
    },
    /// Re-launch a frozen blueprint
    Up { name: String },
    /// Interactive session switcher (live + snapshot + zoxide)
    Switch,
    /// Session management actions (sessionist-style)
    Session {
        #[command(subcommand)]
        action: SessionCmd,
    },
    /// Print the status-bar widget text
    Status,
    /// Run the interval auto-save daemon
    Daemon,
    /// Internal: event-driven save trigger from tmux hooks
    #[command(hide = true)]
    Hook { event: String },
    /// Internal: restore-on-start guard
    #[command(hide = true)]
    Autostart,
}

#[derive(Subcommand)]
pub enum SessionCmd {
    /// Create or switch to a named session
    New { name: String },
    /// Kill the current session, switching away first
    Kill,
    /// Move the current pane into a new session
    Promote { name: String },
    /// Switch to a session by name
    Switch { name: String },
    /// Switch to the last session
    Last,
    /// Rename the current session
    Rename { name: String },
}
