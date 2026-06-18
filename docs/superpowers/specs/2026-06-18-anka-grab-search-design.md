# anka grab + anka search — fold extrakto + fuzzback into anka

Status: approved 2026-06-18.

## Goal

Replace the `laktak/extrakto` and `roosta/tmux-fuzzback` plugins with two native
anka subcommands that reuse anka's picker, pane-capture, URL handling, and
clipboard:

- **`anka grab`** (extrakto) — extract tokens from the pane (url / path / word /
  line, filter-cycled), then copy / paste / open.
- **`anka search`** (fuzzback) — fuzzy-search the scrollback, jump to the chosen
  line in copy-mode.

After this, the tmux plugin set is **3**: tpm, anka, huma.

## Entry points

Two subcommands, each launched like `anka url` (a `run-shell` bind captures
`#{pane_id}` — formats expand there, not in `display-popup -E` — anka captures the
pane text to a temp file, then reopens itself in a popup running the picker;
actions target the original pane via `-t <pane>`):

- `prefix+Space` → `anka grab` (key `@anka-grab-key`, default `Space`)
- `prefix+m` → `anka search` (key `@anka-search-key`, default `m`)

Bind form: `run-shell -b "<bin>/anka grab --pane '#{pane_id}'"`. anka captures,
writes `$TMPDIR/anka-grab-<pane>.txt`, opens `display-popup -E "<bin>/anka grab
<tmpfile> --pane <pane>"`. The popup step reads the file, runs the picker, acts.

## Picker enhancement (`src/picker.rs`)

The shared picker gains an extended entry for grab's multi-action + filter-cycle
UX, without changing `pick`/`pick_str`:

- `Key::Ctrl(char)` added to the switcher `Key` enum; `parse_key` maps control
  bytes `0x01..=0x1a` (except the already-mapped ^n/^p/^r/^x/^c/tab/cr) to
  `Key::Ctrl((b + 0x60) as char)`. switcher/url/menu ignore `Ctrl(_)` (no change).
- One shared loop `run_picker(items, title, full) -> Hit` with
  `enum Hit { Enter(usize), Ctrl(char, usize), Tab, Cancel }`.
  - `pub fn pick(items, title) -> Option<usize>` calls `run_picker(.., false)`:
    only Enter/Digit accept; Tab/Ctrl ignored (loop continues). Behaviour
    identical to today.
  - `pub fn pick_ex(items, title) -> Result<Hit>` calls `run_picker(.., true)`:
    Enter / Ctrl(c) / Tab / Cancel all return.
  - `pick_str` unchanged (wraps `pick`).

## `anka grab` (extrakto)

Capture: `capture-pane -p -J -t <pane> -S -100` (visible + ~100 lines scrollback).

Filters (cycled with **Tab**, order `all url path word line`, current filter in
the picker title):

- **url** — `url::extract_urls` (already handles http/https/ftp/file/mailto, www.,
  scheme-less `host.tld/path`, and GitHub `owner/repo`).
- **path** — tokens that look like filesystem paths: start with `/`, `~/`, `./`,
  `../`, or contain `/` plus a filename with an extension. Pure `parse_paths`.
- **word** — whitespace-split tokens of length ≥ 2 (trim surrounding
  punctuation). Pure `parse_words`.
- **line** — non-empty whole lines (trimmed). Pure `parse_lines`.
- **all** — url ++ path ++ word, de-duplicated in that priority order.

Actions on the selected token (`pick_ex`):

- **Enter → copy** via `clip::copy(token, primary=false)`.
- **^v → paste** into the original pane: `send-keys -t <pane> -l <token>`.
- **^o → open** via `url::open(token)` (browser / `@anka-url-browser`).
- **Tab → cycle** the filter; **Esc/^c → cancel**.

Reuse: `clip::copy` and `url::open` are made `pub(crate)` (extract `clip::copy`
from `clip::run`; expose `url::open`). Empty capture → `display-message` + exit.

## `anka search` (fuzzback)

Capture: `capture-pane -p -J -t <pane> -S -` (full scrollback). Enumerate lines
top→bottom (index 0 = oldest). Picker shows non-empty lines (label trimmed),
newest first for convenience but each item keeps its absolute line index.

On select → jump: `copy-mode -t <pane>` then
`send-keys -X -t <pane> goto-line <index>`. A pure `goto_line_target(total,
selected_top_index)` returns the index handed to `goto-line`. Column precision
(positioning the cursor at the match within the line) is **out of scope for v1**.

## Components

- `src/picker.rs` — `run_picker` + `pick_ex` + `Hit` (above); `pick`/`pick_str`
  refactored onto `run_picker`.
- `src/switcher/state.rs` + `term.rs` — add `Key::Ctrl(char)` + the parse arm.
- `src/grab.rs` — capture, the pure extractors (`parse_paths`, `parse_words`,
  `parse_lines`, `tokens_for(filter, text)`), filter cycling, the popup flow,
  and the three actions.
- `src/search.rs` — capture, `goto_line_target` (pure), the popup flow + jump.
- `src/clip.rs` — factor `pub(crate) fn copy(data: &[u8], primary: bool)` out of
  `run`.
- `src/url.rs` — make `open` `pub(crate)`.
- `src/cli.rs` / `src/main.rs` — `Grab { pane, source }` and `Search { pane,
  source }` subcommands (mirror `Url`).
- `anka.tmux` — bind `@anka-grab-key` / `@anka-search-key` (skip on `none`).

## Error handling

- No tokens / empty scrollback → `display-message` ("anka: nothing to grab" /
  "no scrollback") and exit, like `anka url`'s "no URLs".
- Cancel at any picker level exits cleanly (raw mode restored).
- Actions never panic on a vanished pane; a failed tmux command surfaces via
  `display-message`.

## Testing

- **Unit:** `parse_paths` / `parse_words` / `parse_lines` / `tokens_for` (filter
  union + dedup + order) against sample pane text; `goto_line_target`; the
  filter-cycle index wrap. Mirror anka's table-style tests.
- **Integration (throwaway tmux server):** `capture-pane` formats return data;
  `goto-line` lands copy-mode on the expected line. The raw-mode picker is
  exercised manually (as with switcher/url/menu).

## Migration (retire extrakto + fuzzback)

1. `anka.tmux`: bind `prefix+Space` → grab, `prefix+m` → search.
2. `.cachy/tmux.conf`: remove the `extrakto` + `tmux-fuzzback` `@plugin` lines and
   their `@extrakto_*` / `@fuzzback-*` settings; keep `@anka-url-browser` (grab's
   open reuses it).
3. Remove both submodules from `.cachy`.
4. Bump anka to **0.12.0**, tag/push; bump `.cachy` gitlink + binary; live-apply.

## Out of scope (YAGNI)

Column-precise jump in search; extrakto's edit-before-insert, multi-select,
configurable grab-area, and `insert+open` combos. Add later if wanted.
