//! Native session switcher: an interactive picker over live + snapshot sessions
//! (and windows / zoxide dirs), split into a pure state machine and thin I/O.

mod state;

pub use state::*;
