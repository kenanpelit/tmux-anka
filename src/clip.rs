//! `anka clip` — copy stdin to the system clipboard, picking the backend from
//! the environment: Wayland (`wl-copy`), X11 (`xclip`), OSC52 to the terminal,
//! else `tmux set-buffer -w`. Used from copy-mode via
//! `copy-pipe-no-clear 'anka clip'`, replacing tmux-yank. Inside tmux,
//! remote/SSH copies are also covered by `set-clipboard on` (OSC52); this
//! handles the local clipboard.

use std::io::{self, Read, Write};
use std::process::{Command, Stdio};

use anyhow::Result;

#[derive(Debug, PartialEq)]
pub enum Backend {
    Wayland,
    X11,
    Osc52,
    /// `tmux set-buffer -w`: the headless/SSH copy-pipe case. copy-pipe runs
    /// us without a controlling terminal, so OSC52-via-/dev/tty silently does
    /// nothing; tmux itself forwards the buffer to the client's clipboard per
    /// its `set-clipboard` setting.
    TmuxBuffer,
}

/// Choose a clipboard backend: wl-copy → xclip → OSC52 (needs /dev/tty) →
/// tmux set-buffer. Without a tty *or* tmux, OSC52 remains the (no-op) last
/// resort, matching the old behavior.
pub fn backend(wayland: bool, x11: bool, tty: bool, tmux: bool) -> Backend {
    if wayland {
        Backend::Wayland
    } else if x11 {
        Backend::X11
    } else if tty || !tmux {
        Backend::Osc52
    } else {
        Backend::TmuxBuffer
    }
}

/// Standard padded base64 (no external crate) — for the OSC52 payload.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// The OSC52 set-clipboard escape sequence for `data`.
pub fn osc52(data: &[u8]) -> String {
    format!("\x1b]52;c;{}\x07", base64(data))
}

fn wl_args(primary: bool) -> Vec<&'static str> {
    if primary {
        vec!["--primary"]
    } else {
        vec![]
    }
}

fn xclip_args(primary: bool) -> Vec<&'static str> {
    if primary {
        vec!["-selection", "primary"]
    } else {
        vec!["-selection", "clipboard"]
    }
}

fn copy_via(cmd: &str, args: &[&str], data: &[u8]) {
    if let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data);
        }
        let _ = child.wait();
    }
}

/// Copy `data` to the clipboard, choosing the backend from the environment.
pub(crate) fn copy(data: &[u8], primary: bool) {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    // Only probe /dev/tty when it could matter (no display server).
    let tty = if wayland || x11 {
        None
    } else {
        std::fs::OpenOptions::new().write(true).open("/dev/tty").ok()
    };
    let in_tmux = std::env::var_os("TMUX").is_some_and(|v| !v.is_empty());
    match backend(wayland, x11, tty.is_some(), in_tmux) {
        Backend::Wayland => copy_via("wl-copy", &wl_args(primary), data),
        Backend::X11 => copy_via("xclip", &xclip_args(primary), data),
        Backend::Osc52 => {
            if let Some(mut tty) = tty {
                let _ = tty.write_all(osc52(data).as_bytes());
            }
        }
        Backend::TmuxBuffer => {
            let text = String::from_utf8_lossy(data);
            crate::tmux::run_ok(&["set-buffer", "-w", "--", &text]);
        }
    }
}

pub fn run(primary: bool) -> Result<()> {
    let mut data = Vec::new();
    io::stdin().read_to_end(&mut data)?;
    copy(&data, primary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_prefers_wayland_then_x_then_osc52() {
        assert_eq!(backend(true, true, true, true), Backend::Wayland);
        assert_eq!(backend(true, false, false, false), Backend::Wayland);
        assert_eq!(backend(false, true, false, false), Backend::X11);
        assert_eq!(backend(false, false, true, false), Backend::Osc52);
    }

    #[test]
    fn headless_copy_pipe_falls_back_to_tmux_buffer() {
        // No display server + no /dev/tty (copy-pipe) + inside tmux → the copy
        // must go through `set-buffer -w`, not silently do nothing.
        assert_eq!(backend(false, false, false, true), Backend::TmuxBuffer);
        // A tty available → OSC52 still wins over set-buffer.
        assert_eq!(backend(false, false, true, true), Backend::Osc52);
        // Neither tty nor tmux: nothing can work; stays OSC52 (harmless no-op).
        assert_eq!(backend(false, false, false, false), Backend::Osc52);
    }

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn osc52_wraps_base64() {
        assert_eq!(osc52(b"x"), "\x1b]52;c;eA==\x07");
    }

    #[test]
    fn xclip_selection_args() {
        assert_eq!(xclip_args(false), vec!["-selection", "clipboard"]);
        assert_eq!(xclip_args(true), vec!["-selection", "primary"]);
    }
}
