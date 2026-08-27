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

### 3. Layouts are one implementation

Three layouts exist and differ only by `BrowserPlacement { Off, Top, Left }`. The table, columns, transport bar, search, selection model and keybinds are shared.

*How to tell you have broken it:* two files contain similar table rendering code. Adding a layout should mean adding an enum variant and a small amount of arrangement code. If it needs a new table widget, it is not a layout and should not be added.

### 4. No user-arranged layouts, ever

No dockable panes, no splitters the user drags, no layout config files, no plugin system. Do not add these even if asked in a way that sounds reasonable. Column show/hide/reorder/resize is allowed and is the only exception.

### 5. `origin` never drives playback

`PlaybackSource.tracks` is what plays. `PlaybackSource.origin` is a display label. Never re-derive the play order from `origin` or from a stored query.

*How to tell you have broken it:* any code path that re-runs a `Selection` to decide what plays next. Shuffle is a permutation held alongside `tracks`, not a reordered query -- if shuffling touches a `Selection`, it is wrong.

### 6. Artwork follows what is playing

Everywhere it is shown by default. The only exception is the explicit right-click "Show album art" action on a specific track. Do not add a follow-selection toggle.

### 7. Album artist is structural, artist is display

Filter panes, grouping, and sorting always use `albumartist` / `albumartistsort`. `artist` is only ever rendered in a column. Never build a filter pane or a group key from `artist`.

### 8. Audio runs on its own OS thread

`std::thread::spawn` plus `std::sync::mpsc`. Never a tokio task, never `async` in the playback path. Tokio is for scanning, tag reading, artwork caching, and database I/O.

### 9. Hardcode the value, not the shape

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
scoria-cli/      the `scoria` binary
scripts/         check-boundaries.sh

scoria-tui/ arrives post-1.0. It is deliberately not an empty crate until then.
```

`widgets/table.rs` is shared by all three layouts and by the queue overlay. A second table implementation anywhere is a bug.

---

## Enforcement

`./scripts/check-boundaries.sh` checks the invariants above that the compiler
cannot express: core's dependency graph is free of UI crates, there is only one
track table, iced `Color` never leaves `theme/`, and raw SQL never leaves `db/`.
CI runs it on every push. Run it before committing; if you add an invariant to
this file, add a check for it there too.

---

## Conventions

**Database access goes through repository traits.** No raw SQL in `playback/` or in any UI crate. This also keeps the door open for reconsidering the storage engine later.

**Paths are stored relative to the library root.** Never persist an absolute path. Resolve against the configured root at read time.

**Sort fields are not columns.** A `SortField` knows its own comparable value and comparator. `Album` sorts on `date` but displays `album`; `AlbumArtist` sorts on `albumartistsort` but displays `albumartist`. Never sort on the displayed string.

**Every sort chain ends in a `TrackId` tiebreaker** so equal-key rows are stable across runs.

**Errors:** `thiserror` in `scoria-core`, `anyhow` at binary boundaries. Never `unwrap()` outside tests. Never `panic!` in the audio thread.

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

When something in `DESIGN.md` turns out to be wrong or infeasible, update the document in the same change. A design doc that drifts from the code is worse than none.
