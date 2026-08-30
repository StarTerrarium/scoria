# CLAUDE.md — Scoria

A library-centric music player for Linux, in Rust. Read `DESIGN.md` before starting any task. This file covers how to work in the repo; `DESIGN.md` covers what is being built and why.

This codebase is intended to read as a reference application. Prefer clear over clever. The person maintaining it is not reviewing every diff, so correctness has to come from structure rather than from review.

---

## Invariants

These do not bend. If a task seems to require breaking one, stop and say so rather than working around it.

### 1. `scoria-core` never depends on a UI crate

Not iced, not ratatui, not winit. If core needs something from the UI, the design is wrong. Check `scoria-core/Cargo.toml` before adding any dependency.

### 2. The UI never calls core internals

Only `Command` in, `Event` out. Both stay serialisable.

*How to tell you have broken it:* a view function calls a method on a library, database, or playback object. If a handler reads `self.library.something()` or `self.player.something()`, that is the violation.

The other direction fails more quietly: a `Command` variant that exists and that
nothing sends. It reads as part of the contract, it is never exercised, and the
next frontend takes it at its word.

### 3. Layouts are one implementation

Three layouts exist and differ only by `BrowserPlacement { Off, Top, Left }`. The table, columns, transport bar, search, selection model and keybinds are shared.

*How to tell you have broken it:* two files contain similar table rendering code. Adding a layout should mean adding an enum variant and a small amount of arrangement code. If it needs a new table widget, it is not a layout and should not be added.

### 4. No user-arranged layouts, ever

**The line is structure, not proportion.**

Banned: anything that lets the user decide *what components exist and where they go*. No edit mode. No adding a pane and choosing what goes in it. No moving the browser from the left to the right by dragging it. No dockable panes, no plugin system, no layout files describing an assembled application. Do not add these even if asked in a way that sounds reasonable -- that is how they arrive.

Allowed: adjusting *how big an existing thing is*. Column widths, the drawer's height, the browser's share of the window. These do not let the user construct anything; they let the user fit what already exists to the screen they actually have.

The earlier version of this rule banned "splitters the user drags" and permitted "column resize" in the same breath, which is a contradiction -- resizing a column *is* dragging a divider. The structure/proportion line is what makes both halves consistent, and it is the line the reasoning in `DESIGN.md` §2 was always drawing.

**Sizes are proportions, never pixels.** A height chosen on one monitor is wrong on every other: 560px is over half a 1080p window and a stamp on a 4K one. Store fractions and let the layout divide the space.

### 5. `origin` never drives playback

`PlaybackSource.tracks` is what plays. `PlaybackSource.origin` is a display label. Never re-derive the play order from `origin` or from a stored query.

*How to tell you have broken it:* any code path that re-runs a `Selection` to decide what plays next. Shuffle is a permutation held alongside `tracks`, not a reordered query -- if shuffling touches a `Selection`, it is wrong.

### 6. Artwork follows what is playing

Everywhere it is shown by default. The only exception is the explicit right-click "Show album art" action on a specific track. Do not add a follow-selection toggle.

### 7. Album artist is structural, artist is display

Filter panes, grouping, and sorting always use `albumartist` / `albumartistsort`. `artist` is only ever rendered in a column. Never build a filter pane or a group key from `artist`.

### 8. Scoria never talks to the internet

Not for artwork, tags, lookups, updates or telemetry. The library on disk is the
only source of truth. `check-boundaries.sh` fails the build if an HTTP or TLS
client appears anywhere in the dependency graph, because that is how it would
arrive -- nobody writes a socket by hand, they add a crate that has one.

D-Bus is exempt and is not an exception: souvlaki talks to the local desktop
(§12), not to a network. MusicBrainz ids are read from tags Picard wrote; Scoria
consumes what MusicBrainz knows and never asks it anything.

*How to tell you have broken it:* the dependency graph gained `reqwest`,
`hyper`, `ureq`, `rustls` or similar. Adding a feature that "just needs to fetch
X" is the shape of the mistake.

### 9. Audio runs on its own OS thread

`std::thread::spawn` plus `std::sync::mpsc`. Never a tokio task, never `async` in the playback path. Tokio is for scanning, tag reading, artwork caching, and database I/O.

### 10. Hardcode the value, not the shape

Deferred features ship with their data shape in place and one hardcoded value:

- `sort: Vec<SortKey>` even though 1.0 only exposes presets
- Themes as a serialisable struct even though 1.0 compiles them in
- Keymap as a data structure even though vim bindings are post-1.0

Never collapse these to `sort_column: String`, a hardcoded colour constant, or match arms on `KeyCode` inside a view.

---

## Stack

Pin these. Do not upgrade a major or minor version as part of an unrelated task.

The single source of truth is `[workspace.dependencies]` in the root `Cargo.toml`.
Member crates inherit with `dep.workspace = true` and never name a version.

```toml
iced      = "0.14"    # API changed substantially across 0.12/0.13/0.14
rodio     = "0.22"
symphonia = "0.5"     # must match what rodio resolves; see below
lofty     = "0.25"
rusqlite  = { version = "0.40", features = ["bundled"] }
notify    = "8"
souvlaki  = "0.8"
ratatui   = "0.30"    # post-1.0
```

**symphonia stays at 0.5 even though 0.6 exists.** rodio 0.22 pulls symphonia 0.5
internally through its own feature flags, so pinning 0.6 here would link two
copies of the decoder into the binary. It is not a direct dependency at all until
gapless trimming needs exact sample counts. Check `cargo tree` before moving it.

MSRV is 1.89, set by lofty. iced needs 1.88 and rodio 1.87.

**iced 0.14 specifically.** Training data and search results contain a lot of 0.12 and 0.13 code, and the APIs are not compatible. If you are unsure whether a pattern is current, check `docs.rs/iced/0.14` rather than writing from memory. Mixed-version iced code is the single most likely failure mode in this repo.

Build requires ALSA dev headers: `libasound2-dev` (Debian/Ubuntu) or `alsa-lib-devel` (Fedora).

---

## Workspace

```
scoria-core/
  src/
    library/     scanner, tag reading, album identity
    db/          schema, migrations, repository traits
    query/       Selection, filter cascade, sort chain
    playback/
      backend/   AudioBackend trait + rodio impl
      engine.rs  owns the audio thread
      source.rs  PlaybackSource, shuffle permutation, repeat
    command.rs
    event.rs
scoria-gui/
  src/
    theme/       semantic role table + iced conversion
    keymap/      data-driven bindings → Action
    widgets/
      table.rs   THE track table; used by every layout and the queue
      panes.rs   filter panes
      transport.rs
    layouts/     arrangement only, no widget definitions
    overlays/
scoria-bin/      the `scoria` binary: picks a frontend, and is the only crate that builds one
scripts/         check-boundaries.sh

scoria-tui/ arrives post-1.0. It is deliberately not an empty crate until then.
```

`widgets/table.rs` is shared by all three layouts and by the queue overlay. A second table implementation anywhere is a bug.

---

## Enforcement

`./scripts/check-boundaries.sh` checks the invariants above that the compiler
cannot express: core's dependency graph is free of UI crates, there is only one
track table, iced `Color` never leaves `theme/`, raw SQL never leaves `db/`, no
panic lint has been `allow`ed outside a test module, every `Column` is offered
in the panel that turns it on, nothing in the dependency graph can reach the
internet, no `pub fn` under `widgets/` is left without a caller, every
`Command` variant has a sender, the application id agrees with the desktop
entry that carries its icon, and no real home directory appears in the source.

The last two are the same rule at two scales: code nobody reaches is a feature
nobody sees. It has happened twice. The now-playing marker `DESIGN.md` §11
requires shipped with a constant, a commit message and 26 passing tests, and
the table never called it. Then four `Command` variants shipped that no
frontend sent -- and unlike the marker, each was a trap for whoever sent one
first (`DESIGN.md` §3.2). Unit tests do not catch either, because they call the
thing directly, which is the seam that broke.

The desktop-identity check is there because a half-finished rename builds
cleanly, passes every test, and shows the compositor's placeholder icon.

The home-directory one is about what leaves the machine. Fixtures need absolute
paths and the obvious one to type is your own; that is how a login name got into
three files and stayed there for 103 commits. Use `/home/someone`. It costs a
history rewrite once anything is published.

CI runs it on every push. Run it before committing; if you add an invariant to
this file, add a check for it there too.

---

## Conventions

**Database access goes through repository traits.** No raw SQL in `playback/` or in any UI crate. This also keeps the door open for reconsidering the storage engine later.

**Paths are stored relative to the library root.** Never persist an absolute path. Resolve against the configured root at read time.

**Sort fields are not columns.** A `SortField` knows its own comparable value and comparator. `Album` sorts on `date` but displays `album`; `AlbumArtist` sorts on `albumartistsort` but displays `albumartist`. Never sort on the displayed string.

**Every sort chain ends in a `TrackId` tiebreaker** so equal-key rows are stable across runs.

**A layer in a `stack!` is always there, present or absent.** iced diffs the
widget tree by type, so returning a `column` when a layer is not needed and a
`stack` when it is changes the type of that node -- and everything beneath it
is rebuilt from scratch, losing its state. In practice that means the table's
scroll offset: the explorer jumped to the top when any overlay opened, when a
column edge was grabbed, and when a track's menu opened. Three separate
versions of one mistake.

So the stack is unconditional and the *layer* is what varies, with
`Space::new().width(0).height(0)` standing in for nothing. `layouts::arrange`,
`widgets/table.rs` and `App::view` all do this; copy one of them.

The symptom is worth recognising, because it does not look like a tree
problem: something scrolls to the top on its own.

**Errors:** `thiserror` in `scoria-core`, `anyhow` at binary boundaries.

**There must be no known way for the application to panic at runtime.** This is
stronger than "no `unwrap()`" and is the standing requirement, not an aspiration:

- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, or
  `process::exit` in production code. All are denied in `[workspace.lints]`.
- No direct indexing or slicing. `v[i]` and `s[a..b]` panic; use `.get()` and
  handle the `None`. String slicing is a live concern rather than a theoretical
  one, because every string here comes from arbitrary tag metadata and a
  non-char-boundary slice on a CJK title panics.
- Division needs an explicit zero guard. No lint catches that.
- Arithmetic on values derived from file metadata or user input uses
  `checked_*` or `saturating_*`. Elsewhere, pick types that cannot overflow.
- A `Mutex` whose holder panicked returns `Err` on `lock()`. Handle the
  `PoisonError`; never `unwrap()` a lock.
- `unsafe` is `forbid`den workspace-wide, not merely discouraged.

When an error genuinely cannot be handled locally, return it. The seam is built
for this: `Event::Error(CoreError)` exists so the core can report a failure to
the UI instead of dying.

A test module may opt out at the top of `mod tests`:
`#![allow(clippy::unwrap_used, clippy::indexing_slicing)]`. Anywhere else, the
boundary script will flag it.

**Tests:** unit tests for query building, sort chains, and album identity resolution. Integration tests for scan-then-query. The audio backend is behind a trait, so playback logic is testable with a mock — do it.

---

## Things not to do

- Do not add a configuration option to resolve a design question. Pick the better default; the design doc explains why the configuration surface is deliberately small.
- Do not add a genre filter pane. `DESIGN.md` §7.2 explains why it cannot work.
- Do not add tabs to the main window. The queue is an overlay for a reason.
- Do not add a follow-selection / follow-playback toggle for artwork.
- Do not make the queue dynamic or query-backed. `DESIGN.md` §6.2.
- Do not silently drop tracks whose files are missing. Show them struck through.
- Do not use `unsafe` anywhere without asking first.
- Do not add a dependency without saying what it is for and what it replaces.
- Do not reach for a web view, Tauri, or Electron under any circumstances.

---

## Working style

Small, focused commits. One concern each.

When a task touches an invariant, say so before writing code rather than after.

When a design decision is genuinely underdetermined, ask rather than guess. There is a documented `DESIGN.md` §14 assumptions list; add to it rather than making a silent choice.

Prefer standard library and existing dependencies over new crates.

`just` runs everything. `just check` is the full gate CI runs; `just` on its own
lists the rest.

When something in `DESIGN.md` turns out to be wrong or infeasible, update the document in the same change. A design doc that drifts from the code is worse than none.
