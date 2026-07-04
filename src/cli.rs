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
    /// Pick a URL from pane text and open it (in $BROWSER / @anka-url-browser)
    Url {
        /// Capture this pane id, then open the picker in a popup (for keybindings)
        #[arg(long)]
        pane: Option<String>,
        /// File with pane text to pick from (default: stdin) — used inside the popup
        source: Option<String>,
        /// Invoking client name (#{client_name}) — the popup opens on it
        #[arg(long)]
        client: Option<String>,
    },
    /// Extract tokens from the pane → copy/paste/open (extrakto-style)
    Grab {
        /// Capture this pane id, then open the picker in a popup (for keybindings)
        #[arg(long)]
        pane: Option<String>,
        /// File with captured pane text — used inside the popup
        source: Option<String>,
        /// Invoking client name (#{client_name}) — the popup opens on it
        #[arg(long)]
        client: Option<String>,
    },
    /// Fuzzy-search the scrollback → jump to the line + highlight (fuzzback-style)
    Search {
        /// Capture this pane id, then open the picker in a popup (for keybindings)
        #[arg(long)]
        pane: Option<String>,
        /// File with captured scrollback — used inside the popup
        source: Option<String>,
        /// Invoking client name (#{client_name}) — the popup opens on it
        #[arg(long)]
        client: Option<String>,
    },
    /// Copy stdin to the system clipboard (wayland/X/OSC52). For copy-pipe.
    Clip {
        /// Use the primary selection instead of the clipboard
        #[arg(long)]
        primary: bool,
    },
    /// Interactive action menu (command/keybinding/process/window/pane)
    Menu {
        /// Internal: run the picker inside the popup (else open the popup)
        #[arg(long)]
        run: bool,
        /// Invoking client name (#{client_name})
        #[arg(long)]
        client: Option<String>,
        /// Invoking session name (#{session_name})
        #[arg(long)]
        session: Option<String>,
    },
    /// Session management actions (sessionist-style)
    Session {
        #[command(subcommand)]
        action: SessionCmd,
    },
    /// Print the status-bar widget text
    Status,
    /// Check the environment/install (PASS/FAIL lines; exit 1 on any FAIL)
    Doctor,
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
    New {
        #[arg(required_unless_present = "prompt")]
        name: Option<String>,
        /// Ask for the name interactively (raw-mode line prompt in a popup;
        /// reads one stdin line off a tty). Empty/ESC cancels silently.
        #[arg(long)]
        prompt: bool,
    },
    /// Kill the current session, switching away first
    Kill,
    /// Move the current pane into a new session
    Promote {
        #[arg(required_unless_present = "prompt")]
        name: Option<String>,
        /// Ask for the name interactively (see `session new --prompt`)
        #[arg(long)]
        prompt: bool,
    },
    /// Switch to a session by name
    Switch {
        #[arg(required_unless_present = "prompt")]
        name: Option<String>,
        /// Ask for the name interactively (see `session new --prompt`)
        #[arg(long)]
        prompt: bool,
    },
    /// Switch to the last session
    Last,
    /// Rename the current session
    Rename { name: String },
}
