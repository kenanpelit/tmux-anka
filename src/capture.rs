//! Save: query tmux, build the model, write JSON + pane contents.

use anyhow::{bail, Result};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::model::*;
use crate::process;
use crate::store;
use crate::tmux;

/// Field separator (US, 0x1f) — avoids tab/comma injection in titles/paths.
/// A raw newline embedded in a free-text field (cwd/title) still tears the
/// `-F` record across physical lines; `records()` below tolerates that.
const US: char = '\u{1f}';

pub fn save(name: Option<&str>) -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    let name = name.unwrap_or(store::DEFAULT_SNAPSHOT);
    store::validate_name(name)?;
    let cfg = Config::load();

    let (snap, content_jobs) = capture(&cfg)?;

    let dir = store::snapshot_dir(name);
    fs::create_dir_all(&dir)?;
    store::clean_stale_tmp(&dir);
    store::clean_stale_tmp(&store::snapshots_dir());
    if cfg.capture_pane_contents {
        write_pane_contents(&content_jobs, &dir, cfg.history_limit);
    }

    let json_path = store::snapshot_json(name);
    store::rotate_generations(&json_path, cfg.save_generations)?;
    let json = serde_json::to_vec_pretty(&snap)?;
    store::write_atomic(&json_path, &json)?;
    store::set_last(name)?;
    prune_stale_panes(&dir, &snap);

    let stamp = chrono::Local::now().format("%H:%M");
    tmux::set_global_option("@anka_status", &format!("✔ {stamp}"));
    println!(
        "saved snapshot '{name}' ({} sessions)",
        snap.sessions.len()
    );
    Ok(())
}

struct PaneRaw {
    session: String,
    window_index: u32,
    pane_index: u32,
    pane_id: String,
    pane_active: bool,
    pid: i32,
    history_size: u64,
    cwd: String,
    command: String,
    title: String,
}

struct WinMeta {
    name: String,
    active: bool,
    layout: String,
}

/// Build the snapshot model plus a list of (pane_id, relative content path)
/// jobs for the caller to capture pane contents.
fn capture(cfg: &Config) -> Result<(Snapshot, Vec<(String, String)>)> {
    let panes = query_panes()?;
    let windows = query_windows()?;

    // session -> window_index -> panes, all index-ordered.
    let mut tree: BTreeMap<String, BTreeMap<u32, Vec<PaneRaw>>> = BTreeMap::new();
    for p in panes {
        tree.entry(p.session.clone())
            .or_default()
            .entry(p.window_index)
            .or_default()
            .push(p);
    }

    let mut content_jobs = Vec::new();
    let mut sessions = Vec::new();

    for (sname, wins) in &tree {
        let mut windows_vec = Vec::new();
        for (widx, plist) in wins {
            let meta = windows.get(&(sname.clone(), *widx));
            let mut panes_vec = Vec::new();
            for p in plist {
                let restore = resolve_restore(cfg, p);
                let contents = if cfg.capture_pane_contents {
                    let rel = format!("panes/{}.txt", p.pane_id.trim_start_matches('%'));
                    content_jobs.push((p.pane_id.clone(), rel.clone()));
                    Some(rel)
                } else {
                    None
                };
                panes_vec.push(Pane {
                    index: p.pane_index,
                    active: p.pane_active,
                    title: p.title.clone(),
                    cwd: p.cwd.clone(),
                    command: p.command.clone(),
                    pid: p.pid,
                    history_size: p.history_size,
                    contents,
                    restore,
                });
            }
            windows_vec.push(Window {
                index: *widx,
                name: meta.map(|m| m.name.clone()).unwrap_or_default(),
                active: meta.map(|m| m.active).unwrap_or(false),
                layout: meta.map(|m| m.layout.clone()).unwrap_or_default(),
                automatic_rename: true,
                panes: panes_vec,
            });
        }
        sessions.push(Session {
            name: sname.clone(),
            windows: windows_vec,
        });
    }

    let client = Client {
        active_session: display("#{client_session}"),
        last_session: display("#{client_last_session}"),
        width: display("#{client_width}").and_then(|s| s.parse().ok()),
        height: display("#{client_height}").and_then(|s| s.parse().ok()),
    };

    let snap = Snapshot {
        schema: SCHEMA,
        anka_version: env!("CARGO_PKG_VERSION").to_string(),
        saved_at: chrono::Local::now().to_rfc3339(),
        client,
        sessions,
    };
    Ok((snap, content_jobs))
}

fn display(fmt: &str) -> Option<String> {
    tmux::run(&["display-message", "-p", fmt])
        .ok()
        .filter(|s| !s.is_empty())
}

fn resolve_restore(cfg: &Config, p: &PaneRaw) -> RestoreAction {
    if p.command == "nvim" || p.command == "vim" {
        // Capture the full argv (the files being edited) so restore reopens
        // them; the `session` strategy may instead resume a Session.vim.
        return RestoreAction {
            kind: RestoreKind::Nvim,
            command: process::foreground_command(p.pid),
        };
    }
    if let Some(full) = process::foreground_command(p.pid) {
        let base = process::base_name(&full);
        if cfg.restore_processes.iter().any(|w| w == base || *w == p.command) {
            return RestoreAction {
                kind: RestoreKind::Process,
                command: Some(full),
            };
        }
    }
    RestoreAction {
        kind: RestoreKind::Shell,
        command: None,
    }
}

/// Split raw `-F`-formatted tmux output into logical records of exactly
/// `fields` US-separated columns each.
///
/// A pane's cwd or title can (rarely, but really) contain a literal newline;
/// tmux's own per-record output framing then tears that single logical
/// record across two or more physical lines, so a naive `.lines()` walk sees
/// a short line (too few fields) followed by a bogus continuation line — and
/// silently drops the pane. Instead: when a physical line splits into fewer
/// than `fields` columns, merge the *next* physical line onto it (rejoined
/// with `\n`, restoring the embedded newline verbatim inside whichever field
/// it fell in) and recount, until the column count is satisfied.
fn records(out: &str, fields: usize) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let mut lines = out.lines();
    while let Some(first) = lines.next() {
        let mut buf = first.to_string();
        loop {
            if buf.split(US).count() >= fields {
                break;
            }
            match lines.next() {
                Some(next) => {
                    buf.push('\n');
                    buf.push_str(next);
                }
                None => break, // ran out of input; whatever we have is final
            }
        }
        let cols: Vec<String> = buf.split(US).map(String::from).collect();
        if cols.len() >= fields {
            result.push(cols);
        }
    }
    result
}

fn query_panes() -> Result<Vec<PaneRaw>> {
    let fmt = [
        "#{session_name}",
        "#{window_index}",
        "#{pane_index}",
        "#{pane_id}",
        "#{pane_active}",
        "#{pane_pid}",
        "#{history_size}",
        "#{pane_current_path}",
        "#{pane_current_command}",
        "#{pane_title}",
    ]
    .join(&US.to_string());
    let out = tmux::run(&["list-panes", "-a", "-F", &fmt])?;
    let mut v = Vec::new();
    for f in records(&out, 10) {
        v.push(PaneRaw {
            session: f[0].clone(),
            window_index: f[1].parse().unwrap_or(0),
            pane_index: f[2].parse().unwrap_or(0),
            pane_id: f[3].clone(),
            pane_active: f[4] == "1",
            pid: f[5].parse().unwrap_or(0),
            history_size: f[6].parse().unwrap_or(0),
            cwd: f[7].clone(),
            command: f[8].clone(),
            title: f[9].clone(),
        });
    }
    Ok(v)
}

fn query_windows() -> Result<BTreeMap<(String, u32), WinMeta>> {
    // `window_name` (free text, user/automatic-rename controlled) is placed
    // last so it plays the same "protected last field" role `pane_title`
    // already does in `query_panes`'s format.
    let fmt = [
        "#{session_name}",
        "#{window_index}",
        "#{window_active}",
        "#{window_layout}",
        "#{window_name}",
    ]
    .join(&US.to_string());
    let out = tmux::run(&["list-windows", "-a", "-F", &fmt])?;
    let mut m = BTreeMap::new();
    for f in records(&out, 5) {
        m.insert(
            (f[0].clone(), f[1].parse().unwrap_or(0)),
            WinMeta {
                active: f[2] == "1",
                layout: f[3].clone(),
                name: f[4].clone(),
            },
        );
    }
    Ok(m)
}

/// Capture each pane's scrollback, limited to `history_limit` lines
/// (`0` = unlimited, the previous unconditional behavior).
fn write_pane_contents(jobs: &[(String, String)], snapshot_dir: &Path, history_limit: u64) {
    let start = if history_limit == 0 {
        "-".to_string()
    } else {
        format!("-{history_limit}")
    };
    for (pane_id, rel) in jobs {
        let content = tmux::run(&["capture-pane", "-p", "-J", "-t", pane_id, "-S", &start])
            .unwrap_or_default();
        let file = snapshot_dir.join(rel);
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(file, content);
    }
}

/// Delete files under `<snapshot_dir>/panes/` that the just-written snapshot
/// no longer references (panes that closed, or scrollback capture toggled
/// off), so they don't accumulate forever.
fn prune_stale_panes(snapshot_dir: &Path, snap: &Snapshot) {
    let referenced: HashSet<&str> = snap
        .sessions
        .iter()
        .flat_map(|s| &s.windows)
        .flat_map(|w| &w.panes)
        .filter_map(|p| p.contents.as_deref())
        .collect();
    let panes_dir = snapshot_dir.join("panes");
    let Ok(rd) = fs::read_dir(&panes_dir) else {
        return;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let rel = format!("panes/{}", entry.file_name().to_string_lossy());
        if !referenced.contains(rel.as_str()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── -F record parser tolerance (fix 12) ─────────────────────────────────

    fn rec(fields: &[&str]) -> String {
        fields.join(&US.to_string())
    }

    #[test]
    fn records_parses_well_formed_lines_normally() {
        let out = format!(
            "{}\n{}",
            rec(&["s", "1", "0", "%1", "1", "100", "0", "/tmp", "bash", "t1"]),
            rec(&["s", "2", "0", "%2", "0", "101", "0", "/home", "zsh", "t2"]),
        );
        let recs = records(&out, 10);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0][3], "%1");
        assert_eq!(recs[1][7], "/home");
    }

    #[test]
    fn records_merges_a_field_containing_an_embedded_newline() {
        // The 8th field (cwd, index 7) contains a literal newline, which tears
        // this single logical record into two physical lines when tmux writes
        // its own record-terminating '\n'. The pane must not be dropped, and
        // the reconstructed field must keep the embedded newline verbatim.
        let good = rec(&["s", "1", "0", "%1", "1", "100", "0", "/tmp", "bash", "title1"]);
        let broken =
            rec(&["s", "1", "1", "%2", "0", "101", "0", "/tmp/weird\nline", "zsh", "title2"]);
        let out = format!("{good}\n{broken}");

        let recs = records(&out, 10);
        assert_eq!(recs.len(), 2, "the newline-splitting pane must not be dropped");
        assert_eq!(recs[1][7], "/tmp/weird\nline");
        assert_eq!(recs[1][8], "zsh");
        assert_eq!(recs[1][9], "title2");
    }

    #[test]
    fn records_does_not_drop_the_record_when_only_the_last_field_has_a_newline() {
        // A newline inside the *last* field (title) is a narrower case: once
        // the column count already reaches `fields`, there's no way to tell
        // whether the last field is genuinely complete or merely paused at an
        // embedded newline, so the remainder is left as leftover text (same
        // as pre-fix). What matters — and regresses without the fix — is that
        // the pane record itself is not silently dropped.
        let broken = rec(&["s", "3", "0", "%3", "1", "102", "0", "/tmp", "bash", "multi\nline title"]);
        let recs = records(&broken, 10);
        assert_eq!(recs.len(), 1, "the pane record itself must not be dropped");
        assert_eq!(recs[0][0], "s");
        assert_eq!(recs[0][3], "%3");
    }

    #[test]
    fn records_drops_a_truly_short_trailing_record() {
        // Not a newline-split case — genuinely malformed/truncated input with
        // no more lines to merge in. Dropped, not looped forever.
        let out = "a\u{1f}b\u{1f}c";
        assert!(records(out, 10).is_empty());
    }

    // ── stale pane pruning (fix 11) ──────────────────────────────────────────

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("anka-capture-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("panes")).unwrap();
        dir
    }

    fn shell_pane(index: u32, contents: Option<&str>) -> Pane {
        Pane {
            index,
            active: false,
            title: String::new(),
            cwd: "/tmp".to_string(),
            command: String::new(),
            pid: 0,
            history_size: 0,
            contents: contents.map(String::from),
            restore: RestoreAction { kind: RestoreKind::Shell, command: None },
        }
    }

    fn snapshot_with_panes(panes: Vec<Pane>) -> Snapshot {
        Snapshot {
            schema: SCHEMA,
            anka_version: "t".to_string(),
            saved_at: "t".to_string(),
            client: Client::default(),
            sessions: vec![Session {
                name: "s".to_string(),
                windows: vec![Window {
                    index: 1,
                    name: "w".to_string(),
                    active: true,
                    layout: String::new(),
                    automatic_rename: true,
                    panes,
                }],
            }],
        }
    }

    #[test]
    fn prune_stale_panes_removes_unreferenced_files_keeps_referenced() {
        let dir = scratch_dir("prune");
        fs::write(dir.join("panes/1.txt"), "keep").unwrap();
        fs::write(dir.join("panes/2.txt"), "stale").unwrap();

        let snap = snapshot_with_panes(vec![shell_pane(0, Some("panes/1.txt"))]);
        prune_stale_panes(&dir, &snap);

        assert!(dir.join("panes/1.txt").exists(), "referenced pane file must survive");
        assert!(!dir.join("panes/2.txt").exists(), "unreferenced pane file must be pruned");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_stale_panes_clears_all_when_contents_disabled() {
        let dir = scratch_dir("prune-disabled");
        fs::write(dir.join("panes/1.txt"), "old").unwrap();
        let snap = snapshot_with_panes(vec![shell_pane(0, None)]);
        prune_stale_panes(&dir, &snap);
        assert!(!dir.join("panes/1.txt").exists());
        fs::remove_dir_all(&dir).ok();
    }
}
