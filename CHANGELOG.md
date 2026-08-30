# Changelog

## 1.0.0

First release.

Scoria is a library-centric music player for Linux. The library is the primary
object rather than a source to build playlists from, and you navigate it
directly.

### Library

- Scans a music folder and watches it: files added, changed or removed are
  picked up without asking. An explicit rescan is always available.
- FLAC, MP3, Ogg Vorbis, AAC and Apple Lossless in MP4, raw AAC, WAV and AIFF.
  A format is indexed only if there is a decoder behind it, enforced by a
  decoding fixture per extension.
- Album identity is resolved by a ladder that falls back through MusicBrainz
  ids, so a compilation does not fragment and two albums of the same name do
  not merge.
- Album artist is structural and artist is display: filter panes, grouping and
  sorting always use `albumartist`.
- Tracks whose files have gone missing are shown struck through rather than
  silently dropped.

### Playback

- Gapless. The next track's decoder is initialised before the current one
  drains.
- ReplayGain read from tags: album gain in album order, track gain when
  shuffled. Never computed, never guessed; untagged tracks play at 0 dB.
- What plays is a materialised, ordered snapshot of track ids. Browsing,
  filtering and re-sorting never disturb it, and shuffle is a permutation held
  alongside it rather than a re-run query.
- Shuffle, repeat off/all/one, and a queue overlay showing the snapshot.
- Media keys, `playerctl` and the desktop's own panel over MPRIS -- from the
  graphical player and from `scoria play` alike.

### Interface

- Three layouts, differing only in where the browser sits: off, on top, or on
  the left. One table implementation behind all of them.
- Cascading filter panes, search, sortable and resizable columns, and a
  configurable column set.
- Transport icons are drawn rather than typed, so no font decides what they
  look like.
- Right-click a track for its properties, its album art, or to filter to its
  artist or album; right-click the column headings for the column panel.
- Every action reachable by a keybinding is also reachable without one. `?`
  lists the bindings, generated from the keymap so it cannot drift.

### The command line

The same binary drives the library from a terminal -- `scoria scan`,
`scoria artists`, `scoria play` -- through the same `Command`/`Event` seam the
graphical frontend uses.

### Constraints

- Scoria never talks to the internet, for anything. The build fails if an HTTP
  or TLS client appears anywhere in the dependency graph.
- No known way to panic at runtime. `unwrap`, `expect`, `panic!`, direct
  indexing and slicing are denied workspace-wide, and `unsafe` is forbidden.
- No web view, and nothing written outside your home directory.
