# Scoria

A library-centric music player for Linux, in Rust.

Most players are playlist-centric and treat the library as a source to build
playlists from. Scoria inverts that: the library is the primary object, and you
navigate it directly. Opinionated defaults, dense information display, minimal
configuration.

It is aimed at someone with a well-organised local library who listens to whole
albums and wants to see a lot of information at once.

> **Status: early development.** The core works -- scanning, querying, gapless
> playback -- and is driven from a command line. There is no graphical frontend
> yet.

## Design

[`DESIGN.md`](DESIGN.md) is the authority on what is being built and why, down to
the decisions that were deliberately *not* taken. Some highlights:

- **Three layouts, one implementation.** They differ by where the browser sits and
  nothing else. There are no dockable panes, no splitters, and no layout files --
  not in 1.0, not later.
- **Playback is a snapshot.** What plays is a materialised, ordered list of track
  ids. Browsing, filtering and re-sorting never disturb it.
- **Album artist is structural; artist is display.** Filter panes, grouping and
  sorting always use `albumartist`; `artist` is only ever rendered in a column.
- **Configuration tunes defaults; it never constructs the application.** The app is
  fully usable by someone who never opens settings.

The library is assumed to be tagged with [MusicBrainz Picard](https://picard.musicbrainz.org/).
That assumption is load-bearing: sorting on `albumartistsort` is what makes
`The Omnific` sort as `Omnific, The` and `花冷え。` as `HANABIE.` without any
special-case code.

## Building

Requires Rust 1.89 or newer and the ALSA development headers:

```sh
# Debian/Ubuntu
sudo apt install libasound2-dev
# Fedora
sudo dnf install alsa-lib-devel
# Arch
sudo pacman -S alsa-lib
```

Then, with [`just`](https://github.com/casey/just):

```sh
just build
just test
just check    # the full gate CI runs: fmt, clippy, tests, invariants
just          # list every recipe
```

Or with cargo directly:

```sh
cargo build --workspace
cargo test --workspace
./scripts/check-boundaries.sh   # architectural invariants CI also enforces
```

## No-panic policy

The application is written to have no known way to panic at runtime. `unwrap`,
`expect`, `panic!`, direct indexing and slicing, and `unsafe` are denied
workspace-wide rather than discouraged, and `scripts/check-boundaries.sh` flags
any local `allow` that reopens one outside a test module. Errors are returned and
surfaced through the event stream instead.

## Trying it

```sh
cargo build --release
./target/release/scoria scan ~/Music
./target/release/scoria artists
./target/release/scoria play "sleep token"
```

`scoria help` lists the rest. The command line drives the same `Command`/`Event`
seam the graphical frontend will, which is what keeps that seam honest -- and it
exists first, so there is no way for UI code to reach past it.

## Layout

| Crate | Role |
|---|---|
| `scoria-core` | Library index, scanner, query engine, playback engine. Depends on no UI crate. |
| `scoria-gui` | iced frontend. |
| `scoria-cli` | Binary; also drives the core directly for debugging. |

`scoria-core` talks to frontends by receiving `Command` and emitting `Event`, both
serialisable. The frontend never calls into core internals. This is enforced by
the dependency graph and by `scripts/check-boundaries.sh`, not by convention.

A `scoria-tui` (ratatui) frontend is planned post-1.0; the seam is what makes it
cheap.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE).
