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
