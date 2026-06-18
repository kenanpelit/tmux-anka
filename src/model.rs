//! Serde model for a snapshot — the on-disk JSON schema.

use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize, Debug)]
pub struct Snapshot {
    pub schema: u32,
    pub anka_version: String,
    pub saved_at: String,
    pub client: Client,
    pub sessions: Vec<Session>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Client {
    pub active_session: Option<String>,
    pub last_session: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Session {
    pub name: String,
    pub windows: Vec<Window>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Window {
    pub index: u32,
    pub name: String,
    pub active: bool,
    pub layout: String,
    pub automatic_rename: bool,
    pub panes: Vec<Pane>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Pane {
    pub index: u32,
    pub active: bool,
    pub title: String,
    pub cwd: String,
    pub command: String,
    pub pid: i32,
    pub history_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    pub restore: RestoreAction,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RestoreAction {
    pub kind: RestoreKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RestoreKind {
    Shell,
    Process,
    Nvim,
}
