# Design document

Scoria — a library-centric music player for Linux, in Rust.

A library-centric music player for Linux. Opinionated defaults, dense information display, minimal configuration.

---

## 1. Motivation

Most modern players are playlist-centric and treat the library as a source to build playlists from. This one inverts that: the library is the primary object, and you navigate it directly.

Players like Fooyin and Deadbeef can be configured into approximately the right shape, but getting there requires assembling the application before you can use it. The goal here is to ship the good arrangement as the default, and to keep the configuration surface small enough that most people never open it.

Target user is someone who has a well-organised local library, listens to whole albums, and wants to see a lot of information at once.

### Non-goals

- Streaming service integration
- Music discovery, recommendations, or online radio
- A playlist manager
- A visually rich "modern" interface at the cost of information density
- Windows or macOS support in 1.0 (nothing should actively prevent it later)

---

## 2. Principles

These are load-bearing. Changes to features are cheap; changes to these are not.

**The library is first-class.** Playlists are a post-1.0 addition built on top, never the organising concept.

**Dense over pretty.** When information density and visual polish conflict, density wins. The one exception is Playback Mode (§9.4), which is explicitly the other thing.

**Configuration tunes defaults; it never constructs the application.** Show/hide a column: yes. Assemble a layout from panes: no. The test is whether the app is fully usable by someone who never opens settings.

**No DIY layouts, ever.** Not in 1.0, not later. DIY layout systems become plugin systems, plugin systems expose internals as API, and at that point the core/UI separation is gone. Fooyin's flexibility and Fooyin's configuration burden are the same feature.

**Layouts are arrangements of shared components, never separate implementations.** Adding a layout means adding an enum variant. If it needs its own table widget, it is not a layout.

**Hardcode the value, not the shape.** Where a feature is deferred but its data shape is knowable, ship the shape now with one hardcoded value. Applies to sort chains, themes, and keymaps.

**The core never knows about the UI.** Commands in, events out, both serialisable. See §3.

---

## 3. Architecture

### 3.1 Workspace

```
scoria-core/     library index, scanner, query engine, playback engine, queue
scoria-gui/      iced frontend
scoria-tui/      ratatui frontend (post-1.0)
scoria-cli/      thin binary, selects frontend
```

`scoria-core` has no dependency on any UI crate. This is enforced by the dependency graph, not by convention.

### 3.2 The seam

The frontend never calls into core internals. It sends commands and consumes events.

```rust
pub enum Command {
    Play(TrackId),
    PlayFrom { source: SourceSpec, at: TrackId },
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek(Duration),
    SetVolume(f32),
    SetPosition(usize),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    Query(QueryId, Selection),
    Rescan,
    SetRoot(PathBuf),
}

pub enum SourceSpec {
    Library { sort: SortChain },                        // "play the whole library"
    Shown { tracks: Vec<TrackId>, label: Option<String> },  // "play what's shown"
    Album(AlbumId),
}

pub enum Event {
    PlaybackStateChanged(PlaybackState),
    TrackChanged { track: TrackId, position: usize },
    PositionChanged(Duration),
    QueueChanged(PlaybackSource),
    QueryResult(QueryId, Vec<TrackId>),
    ScanProgress { seen: usize, total: Option<usize> },
    LibraryChanged(LibraryDelta),
    Error(CoreError),
}
```

`SourceSpec` is how §6.3's play-behaviour setting reaches the core. `Shown`
carries the track list explicitly rather than a `Selection`, because the view
already has the answer -- handing it over means the core never re-runs a query
to decide what plays, which states invariant 5 as a type rather than as a rule.

`Stop` exists because MPRIS defines a Stop verb that desktop controls and
`playerctl` will send whether or not we want one, and answering it by pausing
would be a lie.

Both enums derive `Serialize`/`Deserialize` from the start. We are not building a daemon in 1.0, but the seam is daemon-shaped, so putting a socket in the middle later is mechanical rather than a rewrite. It is also what makes the TUI cheap.

### 3.3 Threading

- **Audio thread.** A dedicated OS thread (`std::thread::spawn`), fed by `std::sync::mpsc`. Never a tokio task. Scheduler jitter that is invisible in a web handler is audible in an audio callback.
- **Async.** Scanning, tag reading, artwork caching, and database I/O run on tokio.
- **UI.** Its own thread, receiving the event stream.

**Failure containment.** The application is written to have no known panic path
(see `CLAUDE.md`), but the audio thread also gets `catch_unwind` at its boundary
as defence in depth. A panic there would otherwise kill audio silently, leaving a
UI that still shows a playing track and a transport that does nothing -- the worst
possible failure presentation. Caught, it becomes `Event::Error` and a stopped
transport, which is at least honest. The same reasoning applies to the scan task.

### 3.4 Audio backend

Behind a trait from day one:

```rust
pub trait AudioBackend: Send {
    fn play(&mut self, track: &TrackRef) -> Result<()>;
    fn preload_next(&mut self, track: &TrackRef) -> Result<()>;
    fn clear_preload(&mut self);
    fn pause(&mut self);
    fn resume(&mut self);
    fn stop(&mut self);
    fn seek(&mut self, pos: Duration) -> Result<()>;
    fn set_gain(&mut self, db: f32);      // per-track ReplayGain, in dB
    fn set_volume(&mut self, linear: f32); // what the user set
    fn position(&self) -> Duration;
    fn finished(&self) -> bool;
}
```

`set_gain` and `set_volume` are separate because they answer different questions: one is a correction the tags asked for and changes per track, the other is what the user set and does not. A backend with native ReplayGain support can honour both. `finished` exists because the engine polls rather than taking a callback -- a callback would run on the backend's own thread, and then two threads would own the snapshot.

**Shuffle and repeat belong to the player, not to a snapshot.** They live on `PlaybackSource` because that is where `advance` needs them, but the engine keeps the preference and stamps it onto each new snapshot. Clicking a different track must not silently turn shuffle off, any more than it should reset the volume.

Default implementation is rodio on Symphonia. The trait costs almost nothing and buys the option to swap in libmpv if pure-Rust gapless on an awkward format becomes a multi-week problem.

**Gapless is not optional.** `preload_next` exists because appending to a rodio `Sink` is queueing, not gapless. The next track's decoder must be initialised and its first packets decoded before the current track's tail drains. FLAC, Vorbis and Opus carry exact sample counts and are straightforward. MP3 requires reading LAME/Info encoder delay and padding and trimming accordingly; that is post-1.0 work (§10).

---

## 4. Data model

### 4.1 Album artist is structural; artist is display

This distinction runs through the entire application.

- `albumartist` sorts, groups, and populates the filter pane. Single clean value: `Electric Callboy`.
- `artist` is only ever a column. Carries full credits: `Electric Callboy feat. Conquer Divide`.

The filter pane is always built from `albumartist`. `artist` never drives structure.

### 4.2 Album identity

The library is assumed to be tagged with MusicBrainz Picard. That makes album identity a lookup rather than a heuristic:

1. `MUSICBRAINZ_ALBUMID` where present.
2. Fall back to `(albumartist, album, date)`.
3. Fall back to containing directory.

Steps 2 and 3 exist for stray untagged files, not as the primary mechanism.

### 4.3 Paths

A single library root (§5.1). Track paths are stored **relative to the root**. Moving the library to another drive or mount point becomes a settings change rather than a full rescan. This is free if done from the start and painful to retrofit.

### 4.4 Sort fields

A sort field is not a column. Sort value and display value are frequently different fields:

| Sort field | Sorts on | Displays as |
|---|---|---|
| Album artist | `albumartistsort` | `albumartist` |
| Album | `date` (chronological) | `album` |
| Track | `(disc, track)` numeric | `track` |

Using `albumartistsort` is why the Picard assumption matters: `The Omnific` sorts as `Omnific, The`, and `花冷え。` sorts as `HANABIE.`. Leading-article handling and Japanese collation both come free instead of needing special cases.

```rust
pub struct SortKey {
    pub field: SortField,
    pub direction: Direction,
}
```

`SortField` knows its own comparable value and comparator. Modelling this as "sort by the string in this column" breaks on the very first key.

Two details: nulls need a defined position, and every chain terminates in a deterministic tiebreaker on `TrackId` so that equal-key rows are stable across restarts and queue snapshots are reproducible.

**Missing values sort last in both directions.** Reversing them along with everything else would mean that flipping a column throws every untagged track to the top, which reads as corruption rather than as an ordering choice. An empty string counts as missing, since a blank tag and an absent one are the same thing to a reader.

The tiebreaker is guaranteed by the `SortChain` constructor rather than by convention, so it cannot be forgotten.

---

## 5. Library and scanning

### 5.1 Single root

One folder. First run is a single "point me at your library" picker.

Multiple roots and named libraries are explicitly out of scope. Music spread across several locations is treated as an edge case; if it turns out not to be, revisit then. This decision removes a library id from the schema, per-root settings, and path disambiguation.

### 5.2 Scanning

- `notify` watches the root continuously.
- An explicit rescan is always available.
- Files that disappear are marked unavailable rather than deleted (§7.4).
- FLAC is the first-class target for 1.0; broad format support is a pre-1.0-release requirement but comes after the FLAC path works end to end.

Tag reading uses `lofty` rather than Symphonia's metadata layer, because tag *writing* is wanted later (§9.3) and lofty covers both.

### 5.3 Artwork

Extracted at scan time, not on demand, because the transport bar wants a thumbnail for whatever starts playing and a decode-on-first-paint stutter is exactly the kind of thing a dense UI shows off.

Precedence: the embedded picture first, then a sidecar in the album directory (`cover.*`, `folder.*`, `front.*`). Embedded wins because it travels with the file and is what Picard writes.

Artwork is cached under `$XDG_CACHE_HOME/scoria/artwork/` keyed by `AlbumId`, stored **exactly as it was found**. Nothing is rescaled.

Rescaling would need an image codec dependency and would buy nothing: artwork always follows what is playing (§9.1), so at most one image is on screen at a time. There is no album grid in this design, and a grid is what would make a thumbnail pipeline worth its weight. The renderer scales one image when it draws it. What the cache does buy is not having to open and parse an audio file to find the picture again, which is the expensive half.

The cache is derived data and safe to delete.

---

## 6. Playback model

The most consequential part of the design.

### 6.1 Browsing never disturbs playback

Non-negotiable. You can filter, search, select and scroll freely while music continues. The transport bar shows what is playing, the table shows what you are looking at, and they are allowed to disagree.

We do not solve this with tabs the way Fooyin does.

### 6.2 The source

```rust
pub struct PlaybackSource {
    tracks: Vec<TrackId>,   // materialised, ordered, immutable once set
    position: usize,
    origin: Origin,         // display metadata only, never drives playback
}

pub enum Origin {
    Library,
    Filtered(String),   // human-readable label, e.g. "Electric Callboy"
    Album(AlbumId),
    Manual,             // arrives with playlists, post-1.0
}
```

`tracks` is what plays. `origin` is a label for the transport bar and the queue header. They answer different questions and are allowed to diverge.

**Why materialised rather than a stored query.** A re-evaluated query has no stable index: a newly imported album that sorts before the current track shifts every subsequent index and silently changes what "next" means mid-album. Retagging a track drops it out of its own filter, leaving position undefined. Changing the table sort would reorder playback. Materialising also gives shuffle a coherent permutation with history, allows drag and remove when playlists arrive, and makes resume-on-restart a deserialisation rather than a re-query that might return something different.

Size is a non-issue: 100,000 `u64` IDs is 800 KB.

### 6.3 Play behaviour setting

The one user-facing option here. Both modes are snapshots; they differ only in what gets snapshotted.

| Setting | Snapshot |
|---|---|
| **Play the whole library** | Entire library, positioned at the clicked track |
| **Play what's shown** (default) | Current view contents, positioned at the clicked track |

One enum, consulted in exactly one place. Everything downstream is identical.

Library mode ignores filters but still respects the table's current sort, since sort is something the user just deliberately set.

### 6.4 Rules

- The snapshot **stops** when it runs out, unless repeat is on. A snapshot that silently expands is not a snapshot; repeat-all wraps the same list rather than extending it, which is why it is allowed to be the one exception.
- On advance, tracks whose files no longer resolve are **skipped**.
- Playing an album from a group header or the album pane snapshots just that album.
- New music added to the library after playback started does **not** join the current queue. It is there next time you press play.

### 6.5 Jump to playing

Restores an unfiltered full-library view and scrolls to the playing track.

Deliberately not scoped to the track's album or artist, because we cannot predict which the user wants. It works anyway: the default sort is album artist → album → disc → track, so landing on the playing track in the unfiltered library automatically places it inside its album, with the artist's other albums immediately above and below. Album and artist context come from the sort order without choosing a filter.

Consequence: `origin` restores nothing. It is purely a label, which makes "origin never drives playback" true by construction rather than by discipline.

### 6.6 Shuffle and repeat

Both operate on the snapshot and never on a query.

Shuffle is a **permutation over `tracks`**, held alongside it. `tracks` itself stays in its natural order and stays immutable, so the queue overlay can show either the running order or the underlying one, and turning shuffle off returns to the natural order without re-deriving anything. Generating the order from a re-run `Selection` would violate §6.2 outright.

`Previous` walks a history of visited positions rather than stepping backwards through the permutation, because in shuffle those are different things and only one of them matches what the user just heard.

Turning shuffle on keeps the current track current -- it moves to the front of the new order rather than being displaced, because shuffling should change what comes *next*, not interrupt what is playing. Turning it off returns to natural order with the same track still playing.

**Toggling shuffle off and on again reshuffles** rather than restoring the previous permutation. Restoring would mean the toggle sometimes reshuffles and sometimes does not, depending on history the user cannot see; always reshuffling is the behaviour that can be predicted from the button alone.

Repeat is `Off | All | One`. `All` wraps at the end of the snapshot; `One` repeats the current position. Neither adds a track that was not in the snapshot. `One` is the engine replaying the same source rather than the snapshot advancing, so `Next` still skips under repeat-one -- the user pressing Next plainly means skip.

### 6.7 ReplayGain

Read from tags at scan time, never computed by analysing audio -- that is a different program's job (`rsgain`, `beets`). Track and album gain and peak are all stored.

`REPLAYGAIN_*` is what 1.0 reads, which covers FLAC, MP3 and any Vorbis-comment file written by the usual tools. Opus and Vorbis may instead carry `R128_*`, in a different unit. That is **not** read yet: `lofty`'s generic tag interface has no escape hatch for arbitrary keys, so reaching them means a per-format code path, and it is not worth one while FLAC is the first-class target (§5.2). It lands with broad format support, before release.

**Which gain applies is derived, not configured:** album gain when playing in album order, track gain when shuffled. Album gain is the default because this is an album-centric player and preserving the loudness relationship *within* a record is the whole point of having album gain at all; shuffle destroys that relationship anyway, so track gain is correct there.

A track with no gain tags plays at 0 dB. Never estimate, never substitute a neighbour's value: a wrong gain is more jarring than none. Peak values are honoured so a positive gain cannot clip.

This is worth stating plainly: the feature is only as good as the library's tag coverage, and a partially tagged library will have audible steps between normalised and un-normalised tracks. The fix is tagging the library, not heuristics in the player.

---

## 7. The library view

### 7.1 Three layouts, one implementation

The three layouts differ on exactly one variable: where the browser sits.

```rust
pub enum BrowserPlacement { Off, Top, Left }
```

The table, columns, transport bar, search field, selection model and keybinds are identical across all three. A keystroke cycles the value.

| Placement | Character |
|---|---|
| `Left` | Panes full height. Many artists visible. Costs ~250 px of table width. |
| `Top` | Panes in a horizontal strip with now-playing artwork. Full table width, ~10 artist rows. |
| `Off` | Single continuous table, album group headers, navigate by search and type-to-jump. |

The claustrophobic feeling in `Top` comes from fewer visible artist rows, not from table width. Sizing the strip to fit the artwork raises it from four rows to about ten, which is what makes that layout viable. The artwork is not decoration filling leftover space; it justifies a strip height the panes need anyway.

### 7.2 Filter panes

Album artist → Album. Each pane has `All (n)` pinned at the top, and counts are contextual, recomputed against the current selection.

**Contextual means upstream-only.** A pane counts against the filters applied *before* it, never its own. Selecting an artist leaves the artist pane listing every artist, and narrows only the album pane below it. A pane whose rows collapsed to the current selection would be a dead end -- there would be no way to change your mind without clearing the filter first. Search sits above both panes and narrows what each counts against.

Every row's count sums to that pane's `All (n)`. That is the contract a pane makes by existing, and it is exactly the contract a genre pane cannot keep.

**No genre pane.** Genre is many-valued per track; Picard writes multiple genre tags from MusicBrainz, so a track can legitimately be several genres at once. A filter pane implies its rows partition the library. Either the counts stop adding to `All`, or we invent a primary-genre rule that is not in the tags. That is a broken contract, not merely a noisy list, and it stays broken in a library tagged only with generic genres.

### 7.3 The track table

One component, used by every layout and by the queue overlay. If it needs a second implementation, something has gone wrong.

**Columns** support show/hide, reorder, and resize, from a right-click on the header. This is configuration that tunes a good default: bounded, discoverable, and ignorable. Splitters, dockable panes and layout files are not.

Default columns: track number, title, duration, album artist, artist, album, year. Album artist and artist both shown by default, deliberately (§4.1).

**Sort** is a `Vec<SortKey>` (§4.4). 1.0 ships presets rather than a chain builder:

- Artist chronological (default) — `albumartistsort`, `date`, `disc`, `track`
- Artist alphabetical
- Recently added
- Album title
- Year

A drag-and-drop chain builder is a DIY layout system wearing different clothes. Users with exotic needs edit the config file.

Clicking a column header applies a **transient override** on top of the configured sort, with a visible indicator and one action to drop back. It never replaces the configured chain. Same relationship the queue snapshot has to the library: a temporary state you can always return from.

**Grouping** is derived, not configured: insert a header row whenever the primary sort key's value changes. It is therefore only meaningful when the chain starts with the grouped field. Sorting by duration switches grouping off automatically. Group headers are click targets for playing the whole album.

### 7.4 Unavailable tracks

Files that vanish are shown struck through with a marker, not removed. Playback skips them (§6.4). Silently dropping rows makes users doubt the software; a struck-through row states what will happen.

---

## 8. Transport bar

Always visible, identical across all three layouts.

Contents: artwork thumbnail, symbol-only transport controls, shuffle and repeat toggles, track title, `artist — album`, jump-to-playing, elapsed/total, volume.

Symbol buttons only, with hover tooltips. Jump-to-playing uses a crosshair/target glyph, matching the "reveal current item" convention from editors and file managers.

Tooltips only solve discovery for mouse users, and this is a keyboard-navigable application. A keybinding cheatsheet on `?` covers the other path.

---

## 9. Overlays

One family, one set of rules. Four overlays with three dismissal behaviours would be its own configuration burden.

- One at a time, never stacked.
- Escape closes. The key that opened it toggles it.
- Anything wanting to be a second simultaneous overlay is a pane, and panes are banned.

### 9.1 Artwork

**Artwork always follows what is playing.** No mode, no toggle. This is a global rule, applied everywhere artwork appears.

Resting state is the transport thumbnail. Clicking it opens a full-size overlay of the playing track's art. In `Top` placement the browser strip also shows it; the thumbnail stays present regardless so the bar does not change shape between layouts.

Escape hatch: right-clicking any track offers "Show album art", which opens the overlay for *that* album without disturbing the ambient rule.

### 9.2 Queue

Modal, on top of the explorer. Modality is the point: being able to see both at once would recreate the ambiguity the separation exists to remove.

Uses the track table pointed at `tracks`. Shows the whole snapshot with the current position marked and scrolled into view, including already-played tracks. Double-click sets `position`.

The queue view exists specifically because `tracks` and `origin` can diverge. Without it there is no authoritative answer to "what plays next", and a `Next` button that disagrees with the visible list reads as a bug.

When the origin filter's current contents differ from the snapshot, show a one-line explanation in the footer. Only when they differ — a permanently present explanation becomes clutter.

### 9.3 Properties

Full metadata dump for a track, scrolling, modal. A complete dump has no editorial layer, so nothing about it needs to be adjacent to the table.

Tag editing is post-1.0 and lands here. An editor needs a save/cancel boundary, which a docked pane that mutates on selection change has nowhere to put. Keeping tag writing behind a deliberate modal action also protects a carefully Picard-tagged library from accidental edits.

### 9.4 Playback mode (post-1.0)

Not a fourth layout. A mode with no table, which is what makes it safe: it shares nothing with the library views and cannot drift into their code.

Cover art, transport, possibly a visualiser. Nothing else. It is the one place where visual polish is the goal, and it exists so the dense views never have to compromise.

Same process, same playback, one keystroke in and the same keystroke out. Not a second window. Resist adding a queue list or a search box; that would make it a fourth layout with all the synchronisation problems that implies.

---

## 10. Keyboard

Arrows and standard shortcuts by default. Vim bindings post-1.0.

**The keymap is a data structure, not match arms in view code.** Two keymaps resolving to one shared action enum — the same enum the TUI will drive. The keymap lives in the UI layer; the actions it resolves to are core commands. Deferring the vim *keymap* is cheap; deferring this *design* would not be.

One collision to resolve: type-to-jump wants bare letters (typing `met` in the artist pane lands on Metallica), and vim normal mode also wants bare letters. They cannot both own unmodified keypresses. Resolution: in vim mode, searching goes through `/`, which is where a vim user's hands go anyway.

---

## 11. Theming

Iced ships 22 built-in theme variants plus `Custom`, and exposes both a base `Palette` and a derived `Extended` palette.

That palette is small — background, text, primary, success, danger, warning — and built for buttons and forms. A dense table needs roles it does not name: selected row (distinct from focused-but-unselected), now-playing row (distinct from selected, since a track can be both, either, or neither), unavailable row, group header background, alternating row tint, column header versus body.

So: **our own semantic role table**, with a function deriving it from an iced `Palette` for the built-ins and per-role overrides where the derivation looks wrong. We get the 22 themes as starting points without being limited to what they name.

**Themes are expressed in our vocabulary, not in iced types.** `Theme` holds `selected_row`, `now_playing`, `text_muted` as plain colours; the iced layer converts at the boundary. This is what lets the TUI consume the same theme definitions with a different converter. Iced `Color` values scattered through view code would force a parallel theme system that drifts immediately.

1.0 ships a curated handful, dark-first, defaulting to dark. **Do not follow the system light/dark preference** — a dense music player that flips to light because the sun went down is startling rather than helpful.

Themes are defined as serialisable data even while compiled in, so user themes later become "also read from a directory" rather than a refactor.

---

## 12. Stack

| Concern | Choice | Note |
|---|---|---|
| Language | Rust | |
| GUI | iced 0.14 | Message architecture matches the core seam; cosmic-text gives system font fallback, which matters for a library full of Japanese titles |
| TUI | ratatui | Post-1.0 |
| Decode | Symphonia 0.5 | Pure Rust, ~±15% of FFmpeg. Pinned to 0.5 because that is what rodio 0.22 resolves; a direct 0.6 dependency would link two copies of the decoder |
| Output | rodio 0.22 → cpal | cpal tries PipeWire → PulseAudio → ALSA |
| Tags | lofty 0.25 | Read and write. Also reads ReplayGain and embedded artwork |
| Database | rusqlite 0.40, `bundled` | Compiles SQLite into the binary |
| Search | SQLite FTS5, `trigram` tokenizer | `unicode61` only matches whole tokens, so `lect` would not find `Electric Callboy`. Trigram matches substrings in any script. Its floor of three characters is covered by a `LIKE` fallback, which matters because two-character queries are ordinary in Japanese |
| Watching | notify | |
| MPRIS | souvlaki 0.8 | Media keys, playerctl, desktop controls |
| Licence | GPL-3.0 | |

**On egui:** viable, and there is prior experience with it, but rejected for two project-specific reasons. It ships no CJK glyphs and needs manual font-chain setup, which is a poor fit for a library where every string comes from arbitrary tag metadata. And immediate mode makes violating the core/UI seam the path of least resistance — `if ui.button("Play").clicked() { self.library.play(id) }` reaches straight through it. Given the code is largely agent-written and not closely reviewed, the framework should make good architecture mandatory rather than aspirational.

**On Turso:** rejected for now. The Rust rewrite is beta, and its headline features (MVCC concurrent writes, vector search, async-first I/O) solve problems a single-user desktop library does not have. `rusqlite` with `bundled` already yields a self-contained binary with no `libsqlite3` dependency, so the "native" gain is mostly dropping the C compiler from the build chain. FTS5 coverage in the rewrite would need verifying before any switch. Keeping data access behind a repository trait makes revisiting this at 1.0 nearly free.

**Runtime dependencies:** libc, libasound, graphics drivers. Build needs ALSA dev headers (`libasound2-dev` / `alsa-lib-devel`).

---

## 13. Scope

### 1.0

Single-root library with watching and rescan. Picard-tagged FLAC as first-class, broad format support before release. Three layouts via `BrowserPlacement`. Filter panes, dense track table with column configuration and sort presets. Snapshot playback with the two-mode setting, shuffle and repeat (§6.6). ReplayGain from tags (§6.7). Scan-time artwork extraction and thumbnail cache (§5.3). Gapless for FLAC/Vorbis/Opus. Queue overlay, artwork overlay, properties overlay (read-only). Transport bar. Arrow-key navigation with a data-driven keymap. Curated dark themes. MPRIS. FTS5 search.

### Post-1.0

Saved playlists. Vim keymap. Tag editing in the properties dialog. Playback Mode. MP3 gapless (LAME/Info delay and padding). User-supplied themes. Custom sort chains beyond presets. Resume-on-restart -- §6.2 cites it as a *reason* the snapshot is materialised, and that argument holds whether or not the feature ships. Computing ReplayGain by analysing audio.

### Never

DIY layouts. A plugin system. Dockable or user-arranged panes.

---

## 14. Assumptions

Recorded because they were decided without explicit sign-off. Flag if any are wrong.

- Sort by `albumartistsort` when present, falling back to `albumartist`.
- Albums under an artist sort chronologically, not alphabetically.
- Virtualisation is designed for a six-figure library regardless of actual size; it is the same work either way.
- GPL-3.0, matching the neighbourhood Fooyin and Deadbeef occupy.
- Scanning both watches and supports explicit rescan.
- Album gain applies in album order and track gain in shuffle, derived rather than configured (§6.7).
- Embedded artwork takes precedence over a sidecar file (§5.3).
- Symphonia stays on whatever version rodio resolves, rather than tracking its own latest.

- Missing sort values land last in both directions (§4.4).
- Settings live in one TOML file at `$XDG_CONFIG_HOME/scoria/config.toml`, with the database under `$XDG_DATA_HOME/scoria/` and thumbnails under `$XDG_CACHE_HOME/scoria/`. Each crate owns its own section of the file, so core never has to name a UI concept in order to store one. A config file that exists but does not parse is an error rather than a silent reset.
- Album identity keys are length-prefixed rather than delimited, so no tag value can forge another album's identity (§4.2).
- The schema is `STRICT`, has no library id anywhere (§5.1), and denormalises album artist and title onto `tracks` because sorting, grouping and the filter cascade all read them per row.
- Shuffle uses a small owned PRNG rather than the `rand` crate, so that shuffle is deterministic under test.

- `LibraryDelta` reports the specific tracks that changed. The scanner knows exactly which they are, so it says so and the UI can avoid a re-query it does not need; `Invalidated` stays available for callers that cannot be precise.
- Artwork is cached at original size, not rescaled (§5.3).
- A file with no title tag is labelled with its filename rather than left blank. This is the only place the scanner fills in a value rather than leaving it absent, and it is a label rather than a claim about the tags.
- Symlinked directories are not followed during a scan, so a library containing a link back to its own root terminates.
- The library is held in memory and queried there; SQLite is the durable store and the full-text index. Going to storage per query costs about half a second at a hundred thousand tracks *whatever the query returns*, because it rebuilds every row. A search box queries per keystroke, so that is the normal path rather than a slow one. Roughly 40 MB at six figures, and `just bench-query` re-measures it.
- Sorting is never delegated to SQL. SQLite's `NOCASE` is ASCII-only, so it would order `TERÄSBETONI` before `Teräsbetoni` where our comparator folds them -- and §6.3 requires a library snapshot to respect the table's sort *exactly*, so two orders would be a correctness bug rather than a cosmetic one.

### Open

Nothing outstanding. Decisions taken during implementation are recorded above.
