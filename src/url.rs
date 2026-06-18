//! `anka url` — pick a URL from captured pane text and open it in the browser.
//!
//! Native replacement for the `capture | extract | fzf | xargs` pipeline: extract
//! URLs ourselves (no regex crate, trailing punctuation trimmed), show them in a
//! small anka-style picker (reusing the switcher's raw-mode terminal layer), and
//! open the chosen one via `$BROWSER`. Falls back to a numbered menu off a tty.

use std::collections::HashSet;
use std::io::{self, IsTerminal, Read, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::switcher::fuzzy_score;

/// Trailing / leading punctuation peeled off a candidate URL.
const TRAIL: &[char] = &[')', '.', ',', ';', ':', '!', '?', ']', '}', '>', '"', '\''];
const LEAD: &[char] = &['(', '[', '{', '<', '"', '\''];

/// Schemes recognised verbatim (kept as-is, no prefix added).
const SCHEMES: &[&str] = &["https://", "http://", "ftp://", "file://", "mailto:"];

/// Extract URLs from text, de-duplicated in first-seen order. Recognises
/// explicit-scheme URLs, `www.` hosts and path-bearing bare domains
/// (`github.com/foo`); the latter two get `https://` prepended. A path is
/// required for scheme-less domains, so bare filenames (`main.rs`,
/// `config.json`) are never mistaken for URLs.
pub fn extract_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for token in text.split_whitespace() {
        if let Some(u) = url_from_token(token) {
            if seen.insert(u.clone()) {
                out.push(u);
            }
        }
    }
    out
}

fn url_from_token(token: &str) -> Option<String> {
    // 1. Explicit scheme anywhere in the token (also peels leading "(", "[a](").
    for scheme in SCHEMES {
        if let Some(pos) = token.find(scheme) {
            let u = token[pos..].trim_end_matches(TRAIL);
            if u.len() > scheme.len() {
                return Some(u.to_string());
            }
        }
    }
    // 2. www. host → assume https.
    if let Some(pos) = token.find("www.") {
        let u = token[pos..].trim_end_matches(TRAIL);
        if u.len() > 4 && u[4..].contains('.') {
            return Some(format!("https://{u}"));
        }
    }
    // 3. Scheme-less, path-bearing domain (host.tld/…).
    let cand = token.trim_start_matches(LEAD).trim_end_matches(TRAIL);
    if is_domain_path(cand) {
        return Some(format!("https://{cand}"));
    }
    // 4. GitHub `owner/repo` shorthand (the tmux `@plugin` form).
    if is_github_shorthand(cand) {
        return Some(format!("https://github.com/{cand}"));
    }
    None
}

/// `owner/repo` GitHub shorthand → github.com/owner/repo. One slash, ASCII slug
/// segments, owner has no dot (domains go through `is_domain_path`), and it isn't
/// an absolute/relative path. Non-ASCII (e.g. Turkish words) is rejected, so
/// prose like `kaydet/yükle` is not mistaken for a repo.
fn is_github_shorthand(s: &str) -> bool {
    if s.starts_with(['/', '.', '~']) {
        return false;
    }
    let Some((owner, repo)) = s.split_once('/') else {
        return false;
    };
    if owner.is_empty() || repo.is_empty() || repo.contains('/') || owner.contains('.') {
        return false;
    }
    let slug = |seg: &str| {
        seg.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    slug(owner) && slug(repo)
}

/// True for `host.tld/path`: a dotted host with an alphabetic TLD and a slash.
fn is_domain_path(s: &str) -> bool {
    let Some(slash) = s.find('/') else {
        return false;
    };
    let host = &s[..slash];
    if !host.contains('.') {
        return false;
    }
    let tld = host.rsplit('.').next().unwrap_or("");
    tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Entry. With `--pane <id>` (the keybinding path): capture that pane and, if it
/// has URLs, reopen ourselves inside a `display-popup` for the picker. Otherwise
/// (inside the popup): read `source`/stdin, extract, pick, open.
pub fn run(pane: Option<&str>, source: Option<&str>) -> Result<()> {
    if let Some(pane_id) = pane {
        return run_pane(pane_id);
    }
    pick_from(source)
}

/// Keybinding path (no tty): capture the pane, then hand off to a popup that
/// runs the picker. `#{pane_id}` only expands in the calling run-shell, which is
/// why capture happens here and not inside the popup.
fn run_pane(pane_id: &str) -> Result<()> {
    let text = crate::tmux::run(&["capture-pane", "-p", "-J", "-t", pane_id, "-S", "-3000"])
        .unwrap_or_default();
    if extract_urls(&text).is_empty() {
        crate::tmux::run_ok(&["display-message", "anka: no URLs in this pane"]);
        return Ok(());
    }
    // Stash the text in a file (keeps the popup's stdin free for keystrokes) and
    // reopen ourselves in the popup for the interactive picker.
    let tmp = std::env::temp_dir().join(format!("anka-url-{}.txt", pane_id.trim_start_matches('%')));
    std::fs::write(&tmp, &text).with_context(|| format!("writing {}", tmp.display()))?;
    let exe = std::env::current_exe()?;
    let cmd = format!("{} url {}", exe.display(), tmp.display());
    crate::tmux::run_ok(&["display-popup", "-w", "70%", "-h", "60%", "-E", &cmd]);
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn pick_from(source: Option<&str>) -> Result<()> {
    let text = match source {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading {path}"))?,
        None => {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s)?;
            s
        }
    };
    let urls = extract_urls(&text);
    if urls.is_empty() {
        println!("no URLs found");
        return Ok(());
    }
    let chosen = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        crate::picker::pick_str(&urls, "urls")?
    } else {
        pick_fallback(&urls)?
    };
    if let Some(url) = chosen {
        open(&url);
    }
    Ok(())
}

/// The URL opener: `@anka-url-browser` (tmux option) → `$BROWSER` → `xdg-open`.
fn browser_cmd() -> String {
    let opt = crate::tmux::global_option("@anka-url-browser");
    if !opt.is_empty() {
        return opt;
    }
    std::env::var("BROWSER").unwrap_or_else(|_| "xdg-open".into())
}

pub(crate) fn open(url: &str) {
    let browser = browser_cmd();
    // Detach (setsid -f) so the browser outlives the closing popup; *wait* for
    // setsid to return so it has reparented the browser into its own session
    // before we exit (otherwise the popup teardown can SIGHUP it).
    let res = Command::new("setsid")
        .arg("-f")
        .arg(&browser)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Ok(s) = &res {
        if s.success() {
            return;
        }
    }
    // Fallback: try the browser directly (no setsid), e.g. setsid missing or the
    // browser is only on a login PATH.
    let _ = Command::new(&browser)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

// ── Non-tty fallback (tests, pipes) ─────────────────────────────────────────

fn pick_fallback(urls: &[String]) -> Result<Option<String>> {
    for (i, u) in urls.iter().enumerate() {
        println!("  {:>2})  {u}", i + 1);
    }
    print!("select [1-{}], a substring, or q: ", urls.len());
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let c = line.trim();
    if c.is_empty() || c.eq_ignore_ascii_case("q") {
        return Ok(None);
    }
    if let Some(n) = c.parse::<usize>().ok().filter(|n| (1..=urls.len()).contains(n)) {
        return Ok(Some(urls[n - 1].clone()));
    }
    Ok(urls
        .iter()
        .filter(|u| fuzzy_score(c, u).is_some())
        .max_by_key(|u| fuzzy_score(c, u).unwrap_or(0))
        .cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_trims_trailing_punctuation() {
        let urls = extract_urls("see (https://github.com/kenanpelit/tmux-anka). and x");
        assert_eq!(urls, vec!["https://github.com/kenanpelit/tmux-anka"]);
    }

    #[test]
    fn handles_markdown_and_dedupes() {
        let urls = extract_urls("[a](https://foo.bar/p) https://foo.bar/p, https://t.co/x,");
        assert_eq!(urls, vec!["https://foo.bar/p".to_string(), "https://t.co/x".to_string()]);
    }

    #[test]
    fn no_urls_is_empty() {
        assert!(extract_urls("nothing here, just text.").is_empty());
    }

    #[test]
    fn finds_http_and_glued() {
        let urls = extract_urls("x=http://a.b/c end");
        assert_eq!(urls, vec!["http://a.b/c"]);
    }

    #[test]
    fn www_gets_https_prefix() {
        assert_eq!(extract_urls("visit www.google.com today"), vec!["https://www.google.com"]);
        assert_eq!(extract_urls("(www.google.com)"), vec!["https://www.google.com"]);
    }

    #[test]
    fn schemeless_domain_with_path() {
        assert_eq!(
            extract_urls("repo github.com/kenanpelit/tmux-anka here"),
            vec!["https://github.com/kenanpelit/tmux-anka"]
        );
    }

    #[test]
    fn bare_filenames_and_pathless_domains_are_not_urls() {
        assert!(extract_urls("edit main.rs and config.json").is_empty());
        assert!(extract_urls("a bare github.com without a path").is_empty());
    }

    #[test]
    fn other_schemes_kept_verbatim() {
        assert_eq!(extract_urls("get ftp://files.x.com/a now"), vec!["ftp://files.x.com/a"]);
    }

    #[test]
    fn github_shorthand_owner_repo() {
        assert_eq!(
            extract_urls("set -g @plugin 'tmux-plugins/tpm'  # mgr"),
            vec!["https://github.com/tmux-plugins/tpm"]
        );
        assert_eq!(
            extract_urls("kenanpelit/tmux-anka and BurntSushi/ripgrep"),
            vec![
                "https://github.com/kenanpelit/tmux-anka".to_string(),
                "https://github.com/BurntSushi/ripgrep".to_string(),
            ]
        );
    }

    #[test]
    fn shorthand_skips_paths_and_prose() {
        // absolute/relative paths are not repos
        assert!(extract_urls("/etc/hosts ./src/main ~/foo/bar").is_empty());
        // non-ASCII prose with a slash is not a repo
        assert!(extract_urls("session kaydet/yükle").is_empty());
        // a real domain shorthand still becomes a domain URL, not github
        assert_eq!(extract_urls("github.com/foo"), vec!["https://github.com/foo"]);
    }
}
