//! Save: query tmux, build the model, write JSON + pane contents.

use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::model::*;
use crate::process;
use crate::store;
use crate::tmux;

/// Field separator (US, 0x1f) — avoids tab/newline injection in titles/paths.
const US: char = '\u{1f}';

pub fn save(name: Option<&str>) -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    let name = name.unwrap_or(store::DEFAULT_SNAPSHOT);
    let cfg = Config::load();

    let (snap, content_jobs) = capture(&cfg)?;

    let dir = store::snapshot_dir(name);
    fs::create_dir_all(&dir)?;
    if cfg.capture_pane_contents {
        write_pane_contents(&content_jobs, &dir);
    }

    let json = serde_json::to_vec_pretty(&snap)?;
    store::write_atomic(&store::snapshot_json(name), &json)?;
    store::set_last(name)?;

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
        return RestoreAction {
            kind: RestoreKind::Nvim,
            command: None,
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
    for line in out.lines() {
        let f: Vec<&str> = line.split(US).collect();
        if f.len() < 10 {
            continue;
        }
        v.push(PaneRaw {
            session: f[0].to_string(),
            window_index: f[1].parse().unwrap_or(0),
            pane_index: f[2].parse().unwrap_or(0),
            pane_id: f[3].to_string(),
            pane_active: f[4] == "1",
            pid: f[5].parse().unwrap_or(0),
            history_size: f[6].parse().unwrap_or(0),
            cwd: f[7].to_string(),
            command: f[8].to_string(),
            title: f[9].to_string(),
        });
    }
    Ok(v)
}

fn query_windows() -> Result<BTreeMap<(String, u32), WinMeta>> {
    let fmt = [
        "#{session_name}",
        "#{window_index}",
        "#{window_active}",
        "#{window_name}",
        "#{window_layout}",
    ]
    .join(&US.to_string());
    let out = tmux::run(&["list-windows", "-a", "-F", &fmt])?;
    let mut m = BTreeMap::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split(US).collect();
        if f.len() < 5 {
            continue;
        }
        m.insert(
            (f[0].to_string(), f[1].parse().unwrap_or(0)),
            WinMeta {
                active: f[2] == "1",
                name: f[3].to_string(),
                layout: f[4].to_string(),
            },
        );
    }
    Ok(m)
}

fn write_pane_contents(jobs: &[(String, String)], snapshot_dir: &Path) {
    for (pane_id, rel) in jobs {
        let content =
            tmux::run(&["capture-pane", "-p", "-J", "-t", pane_id, "-S", "-"]).unwrap_or_default();
        let file = snapshot_dir.join(rel);
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(file, content);
    }
}
