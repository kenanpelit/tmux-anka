//! Serde model for a snapshot — the on-disk JSON schema.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

/// Newest snapshot schema this binary knows how to read. A snapshot written
/// by a newer anka (`schema` > this) is refused with a clear message instead
/// of dying deep inside serde with a raw "missing field" error.
pub const SUPPORTED_SCHEMA: u32 = SCHEMA;

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
    /// The active client's terminal size at save time (`#{client_width}` /
    /// `#{client_height}`), used to size the first restored window. Absent
    /// when there was no attached client, or in a snapshot written before
    /// this field existed — restore falls back to a fixed default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// The `{schema, anka_version}` header of a snapshot/blueprint, parsed before
/// the full structure so a too-new file fails with a clear message rather
/// than a raw serde error partway through an unrecognized shape.
#[derive(Deserialize)]
struct SchemaHeader {
    schema: u32,
    #[serde(default)]
    anka_version: String,
}

/// Parse a snapshot/blueprint's bytes, gated on schema.
pub fn parse_snapshot(data: &[u8]) -> Result<Snapshot> {
    let header: SchemaHeader =
        serde_json::from_slice(data).context("parsing snapshot header")?;
    if header.schema > SUPPORTED_SCHEMA {
        let ver = if header.anka_version.is_empty() {
            "?".to_string()
        } else {
            header.anka_version
        };
        bail!(
            "snapshot written by anka {ver} (schema {}); this binary supports schema {SUPPORTED_SCHEMA} — upgrade anka or re-save",
            header.schema
        );
    }
    serde_json::from_slice(data).context("parsing snapshot")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_valid_snapshot_json() -> Vec<u8> {
        let snap = Snapshot {
            schema: SCHEMA,
            anka_version: "0.12.7".to_string(),
            saved_at: "2024-01-01T00:00:00+00:00".to_string(),
            client: Client::default(),
            sessions: vec![],
        };
        serde_json::to_vec(&snap).unwrap()
    }

    #[test]
    fn current_schema_parses_normally() {
        let data = minimal_valid_snapshot_json();
        let snap = parse_snapshot(&data).expect("current schema must parse");
        assert_eq!(snap.schema, SCHEMA);
    }

    #[test]
    fn future_schema_fails_with_clear_upgrade_message() {
        // Doctored snapshot: schema far ahead of what this binary supports,
        // and (realistically) missing fields this older binary doesn't know
        // about yet. Must fail with the clear upgrade message, not a raw
        // serde "missing field" error.
        let data = br#"{"schema":999,"anka_version":"9.9.9"}"#;
        let err = parse_snapshot(data).unwrap_err().to_string();
        assert!(err.contains("schema 999"), "{err}");
        assert!(err.contains("anka 9.9.9"), "{err}");
        assert!(err.contains(&format!("schema {SUPPORTED_SCHEMA}")), "{err}");
        assert!(err.contains("upgrade anka or re-save"), "{err}");
    }

    #[test]
    fn future_schema_with_missing_version_shows_placeholder() {
        let data = br#"{"schema":42}"#;
        let err = parse_snapshot(data).unwrap_err().to_string();
        assert!(err.contains("anka ?"), "{err}");
    }
}
