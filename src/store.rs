//! Snapshot storage: directory layout, the `last` symlink, list/rm.

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs as unixfs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::tmux;

pub const DEFAULT_SNAPSHOT: &str = "default";

/// `@anka-dir`, or the first of: the platform data dir (the existing choice,
/// `${XDG_DATA_HOME:-~/.local/share}/tmux/anka` — kept first so nobody's
/// existing store silently relocates), `$HOME/.local/share/anka`, `/tmp/anka`.
/// The last two only kick in on unusual systems where the `dirs` crate can't
/// resolve a data dir (no resolvable home) — snapshots silently living in
/// `/tmp` (gone on reboot) would otherwise defeat the whole plugin.
pub fn base_dir() -> PathBuf {
    let configured = tmux::global_option("@anka-dir");
    if !configured.is_empty() {
        return expand(&configured);
    }
    default_base_dir_from(
        dirs::data_dir(),
        std::env::var_os("HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
    )
}

fn default_base_dir_from(data_dir: Option<PathBuf>, home_env: Option<PathBuf>) -> PathBuf {
    if let Some(data) = data_dir {
        return data.join("tmux").join("anka");
    }
    if let Some(home) = home_env {
        return home.join(".local").join("share").join("anka");
    }
    PathBuf::from("/tmp").join("anka")
}

/// Expand a leading `~/` or any `$HOME` occurrence using the real home dir;
/// left verbatim (as an absolute/relative path) otherwise.
fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if p.contains("$HOME") {
        if let Some(home) = dirs::home_dir() {
            return PathBuf::from(p.replace("$HOME", &home.to_string_lossy()));
        }
    }
    PathBuf::from(p)
}

pub fn snapshots_dir() -> PathBuf {
    base_dir().join("snapshots")
}

pub fn snapshot_dir(name: &str) -> PathBuf {
    snapshots_dir().join(name)
}

pub fn snapshot_json(name: &str) -> PathBuf {
    snapshot_dir(name).join("snapshot.json")
}

pub fn last_link() -> PathBuf {
    snapshots_dir().join("last")
}

/// The name the `last` symlink points at, if any.
pub fn last_name() -> Option<String> {
    let target = fs::read_link(last_link()).ok()?;
    target
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
}

/// Point `last` at `name` (atomic replace, unique tmp name per writer).
pub fn set_last(name: &str) -> Result<()> {
    let link = last_link();
    let tmp = snapshots_dir().join(format!(".last.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    unixfs::symlink(name, &tmp).context("creating last symlink")?;
    fs::rename(&tmp, &link).context("activating last symlink")?;
    Ok(())
}

pub fn list_names() -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(snapshots_dir()) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

pub fn list_cmd() -> Result<()> {
    let last = last_name();
    let names = list_names();
    if names.is_empty() {
        println!("(no snapshots)");
        return Ok(());
    }
    for name in names {
        let marker = if Some(&name) == last.as_ref() {
            " (last)"
        } else {
            ""
        };
        println!("{name}{marker}");
    }
    Ok(())
}

pub fn rm_cmd(name: &str) -> Result<()> {
    validate_name(name)?;
    let dir = snapshot_dir(name);
    if !dir.is_dir() {
        bail!("snapshot '{name}' not found");
    }
    fs::remove_dir_all(&dir).with_context(|| format!("removing snapshot '{name}'"))?;
    if last_name().as_deref() == Some(name) {
        let _ = fs::remove_file(last_link());
    }
    println!("removed snapshot '{name}'");
    Ok(())
}

/// Reject a user-supplied snapshot/blueprint name that would escape the store
/// dir, land on a dotfile, or collide with the `last` symlink's own name.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("snapshot name must not be empty");
    }
    if name == "last" {
        bail!("'last' is a reserved name (it's the last-saved snapshot symlink) — use the snapshot's real name");
    }
    if name.contains('/') || name.contains("..") || name.starts_with('.') {
        bail!("invalid snapshot name '{name}': must not contain '/', '..', or start with '.'");
    }
    Ok(())
}

/// Write bytes atomically (unique tmp + rename in the same directory), so two
/// writers (daemon tick + client-detached hook) never share one tmp path and
/// interleave/truncate each other's output.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = tmp_path(path);
    if let Err(e) = fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display())) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) =
        fs::rename(&tmp, path).with_context(|| format!("activating {}", path.display()))
    {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// A unique-per-process tmp path alongside `path` (`<path>.<pid>.tmp`).
fn tmp_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    path.with_file_name(format!("{name}.{}.tmp", std::process::id()))
}

/// Remove leftover `*.tmp` files from writers that no longer exist (crashed
/// autosave, killed hook, …) so they don't pile up forever. A tmp file whose
/// embedded pid is still alive is left alone — it may be a save genuinely in
/// flight right now (daemon tick racing a client-detached hook).
pub fn clean_stale_tmp(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".tmp") {
            continue;
        }
        if tmp_owner_pid(&name).is_some_and(pid_alive) {
            continue;
        }
        let _ = fs::remove_file(entry.path());
    }
}

fn tmp_owner_pid(file_name: &str) -> Option<u32> {
    file_name.strip_suffix(".tmp")?.rsplit('.').next()?.parse().ok()
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Rotate `path` → `path.1` → `path.2` … keeping at most `generations` backups
/// before it gets overwritten by a new save. A no-op if `path` doesn't exist
/// yet (first save) or `generations` is 0 (disabled). These are manual-recovery
/// copies only — `restore`/`list` always use `path` itself.
pub fn rotate_generations(path: &Path, generations: u32) -> Result<()> {
    if generations == 0 || !path.is_file() {
        return Ok(());
    }
    let oldest = numbered(path, generations);
    let _ = fs::remove_file(&oldest);
    for gen in (1..generations).rev() {
        let from = numbered(path, gen);
        if from.is_file() {
            fs::rename(&from, numbered(path, gen + 1)).context("rotating snapshot generation")?;
        }
    }
    fs::rename(path, numbered(path, 1)).context("rotating snapshot generation")?;
    Ok(())
}

fn numbered(path: &Path, n: u32) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// Directory for transient, sensitive scratch files (captured pane text): a
/// private `$XDG_RUNTIME_DIR` when set, else `<anka-dir>/tmp`.
pub fn runtime_tmp_dir() -> PathBuf {
    runtime_tmp_dir_from(std::env::var("XDG_RUNTIME_DIR").ok().as_deref())
}

fn runtime_tmp_dir_from(xdg_runtime: Option<&str>) -> PathBuf {
    match xdg_runtime.filter(|s| !s.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => base_dir().join("tmp"),
    }
}

/// Create a private (owner-only, 0600) scratch file with a unique name under
/// `runtime_tmp_dir()`. Uses `create_new` (`O_EXCL`) so a pre-planted symlink
/// or file at the chosen name is never followed/overwritten — on a collision
/// (or a stale file left by a previous run) a new unique name is tried instead.
pub fn create_secure_tmp(prefix: &str) -> Result<(PathBuf, fs::File)> {
    let dir = runtime_tmp_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    sweep_dead_scratch(&dir);
    create_secure_tmp_in(&dir, prefix)
}

/// Sweep `anka-*-<pid>-<n>.txt` scratch files whose owning process is gone.
/// The popup deletes its own file on the normal path, but a killed popup (or
/// tmux server exit mid-pick) leaks one; the next invocation cleans up here.
fn sweep_dead_scratch(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name.strip_prefix("anka-").and_then(|n| n.strip_suffix(".txt")) else {
            continue;
        };
        let mut it = stem.rsplit('-');
        let (Some(_counter), Some(pid)) = (it.next(), it.next()) else { continue };
        if pid.parse::<u32>().ok().is_some_and(|p| !pid_alive(p)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn create_secure_tmp_in(dir: &Path, prefix: &str) -> Result<(PathBuf, fs::File)> {
    use std::os::unix::fs::OpenOptionsExt;
    let pid = std::process::id();
    for counter in 0..1000u32 {
        let path = dir.join(format!("{prefix}-{pid}-{counter}.txt"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
    bail!("could not create a unique temp file in {}", dir.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── name validation (fix 10) ────────────────────────────────────────────

    #[test]
    fn validate_name_accepts_ordinary_names() {
        assert!(validate_name("default").is_ok());
        assert!(validate_name("work-2024_01").is_ok());
    }

    #[test]
    fn validate_name_rejects_path_traversal() {
        assert!(validate_name("../x").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("/etc/passwd").is_err());
        assert!(validate_name("a/../../b").is_err());
    }

    #[test]
    fn validate_name_rejects_dotfiles_reserved_and_empty() {
        assert!(validate_name(".hidden").is_err());
        assert!(validate_name("last").is_err());
        assert!(validate_name("").is_err());
    }

    // ── expand / base_dir fallback (fix 9) ─────────────────────────────────

    #[test]
    fn expand_tilde_and_home_var() {
        let home = dirs::home_dir().expect("HOME must be set for this test");
        assert_eq!(expand("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(expand("$HOME/foo"), home.join("foo"));
    }

    #[test]
    fn expand_leaves_absolute_paths_alone() {
        assert_eq!(expand("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn default_base_dir_keeps_the_existing_data_dir_choice_first() {
        // Backward compatible: existing stores must not silently relocate.
        let got =
            default_base_dir_from(Some(PathBuf::from("/data")), Some(PathBuf::from("/home/u")));
        assert_eq!(got, PathBuf::from("/data/tmux/anka"));
    }

    #[test]
    fn default_base_dir_falls_back_to_home_share() {
        let got = default_base_dir_from(None, Some(PathBuf::from("/home/u")));
        assert_eq!(got, PathBuf::from("/home/u/.local/share/anka"));
    }

    #[test]
    fn default_base_dir_last_resort_is_tmp_not_bare() {
        // Never a bare, unqualified `/tmp` — always namespaced under it.
        let got = default_base_dir_from(None, None);
        assert_eq!(got, PathBuf::from("/tmp/anka"));
    }

    // ── write_atomic / tmp naming (fix 1) ───────────────────────────────────

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anka-store-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tmp_path_is_unique_per_process_and_not_the_old_fixed_name() {
        let target = Path::new("/anywhere/snapshot.json");
        let tmp = tmp_path(target);
        let pid = std::process::id().to_string();
        assert!(tmp.to_string_lossy().ends_with(".tmp"));
        assert!(tmp.to_string_lossy().contains(&pid), "tmp path must embed the pid: {tmp:?}");
        assert_ne!(tmp, target.with_extension("json.tmp"), "must not reuse the old shared fixed tmp name");
    }

    #[test]
    fn write_atomic_activates_and_leaves_no_tmp_behind() {
        let dir = scratch_dir("write-atomic");
        let path = dir.join("snapshot.json");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        let leftover_tmp = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!leftover_tmp, "write_atomic left a tmp file behind");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clean_stale_tmp_removes_dead_pid_but_keeps_live_pid() {
        let dir = scratch_dir("clean-stale");
        // u32::MAX is always well past Linux's pid_max ceiling: never a live pid.
        let dead = dir.join(format!("snapshot.json.{}.tmp", u32::MAX));
        let live = dir.join(format!("snapshot.json.{}.tmp", std::process::id()));
        fs::write(&dead, b"x").unwrap();
        fs::write(&live, b"x").unwrap();
        clean_stale_tmp(&dir);
        assert!(!dead.exists(), "tmp owned by a dead pid should be swept");
        assert!(live.exists(), "tmp owned by a live pid must be left alone");
        fs::remove_dir_all(&dir).ok();
    }

    // ── snapshot generations (fix 2) ─────────────────────────────────────────

    #[test]
    fn rotate_generations_is_noop_on_first_save() {
        let dir = scratch_dir("rotate-first");
        let path = dir.join("snapshot.json");
        rotate_generations(&path, 3).unwrap();
        assert!(fs::read_dir(&dir).unwrap().next().is_none(), "nothing to rotate yet");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotate_generations_is_noop_when_disabled() {
        let dir = scratch_dir("rotate-disabled");
        let path = dir.join("snapshot.json");
        fs::write(&path, "v1").unwrap();
        rotate_generations(&path, 0).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1");
        assert!(!numbered(&path, 1).exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotate_generations_shifts_and_drops_oldest_at_depth() {
        let dir = scratch_dir("rotate-depth");
        let path = dir.join("snapshot.json");
        // Simulate 4 successive saves with generations=3: only the newest 3
        // prior versions should ever be kept as backups.
        for v in 1..=4 {
            fs::write(&path, format!("v{v}")).unwrap();
            if v < 4 {
                rotate_generations(&path, 3).unwrap();
                fs::write(&path, format!("v{}", v + 1)).unwrap();
            }
        }
        // After the loop: path = v4 (current), .1 = v3, .2 = v2, .3 = v1.
        // Do one more explicit rotate to check steady-state depth handling.
        rotate_generations(&path, 3).unwrap();
        fs::write(&path, "v5").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "v5");
        assert_eq!(fs::read_to_string(numbered(&path, 1)).unwrap(), "v4");
        assert_eq!(fs::read_to_string(numbered(&path, 2)).unwrap(), "v3");
        assert_eq!(fs::read_to_string(numbered(&path, 3)).unwrap(), "v2");
        assert!(!numbered(&path, 4).exists(), "must not keep more than `generations` backups");
        fs::remove_dir_all(&dir).ok();
    }

    // ── secure tmp files (fix 8) ─────────────────────────────────────────────

    #[test]
    fn create_secure_tmp_sets_owner_only_permissions() {
        let dir = scratch_dir("secure-mode");
        let (path, _f) = create_secure_tmp_in(&dir, "mode").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secure tmp file must be owner-only (0600)");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_secure_tmp_does_not_follow_a_planted_symlink() {
        let dir = scratch_dir("secure-symlink");
        let pid = std::process::id();
        let planted = dir.join(format!("t-{pid}-0.txt"));
        let victim = dir.join("victim.txt");
        unixfs::symlink(&victim, &planted).unwrap();

        let (path, mut f) = create_secure_tmp_in(&dir, "t").unwrap();
        use std::io::Write;
        f.write_all(b"secret").unwrap();

        assert_ne!(path, planted, "must skip the pre-planted name rather than open through it");
        assert!(!victim.exists(), "must never follow the planted symlink to its target");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sweep_dead_scratch_removes_dead_owner_files_only() {
        let dir = scratch_dir("sweep-scratch");
        let dead = dir.join(format!("anka-url-{}-0.txt", u32::MAX));
        let live = dir.join(format!("anka-grab-{}-0.txt", std::process::id()));
        let unrelated = dir.join("notes.txt");
        fs::write(&dead, "x").unwrap();
        fs::write(&live, "x").unwrap();
        fs::write(&unrelated, "x").unwrap();
        sweep_dead_scratch(&dir);
        assert!(!dead.exists(), "dead-owner scratch file must be swept");
        assert!(live.exists(), "live-owner scratch file must survive");
        assert!(unrelated.exists(), "non-anka files must never be touched");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn runtime_tmp_dir_prefers_xdg_runtime_dir() {
        assert_eq!(runtime_tmp_dir_from(Some("/run/user/1000")), PathBuf::from("/run/user/1000"));
    }

    #[test]
    fn runtime_tmp_dir_falls_back_to_store_tmp() {
        assert_eq!(runtime_tmp_dir_from(None), base_dir().join("tmp"));
        assert_eq!(runtime_tmp_dir_from(Some("")), base_dir().join("tmp"));
    }
}
