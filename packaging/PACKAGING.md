# Packaging

The decision and its reasoning are in [`DESIGN.md`](../DESIGN.md) §15: Flatpak
primary, AUR secondary, no AppImage. This file is the recipe for each.

## Blocked on

Neither channel can be finished yet, and both are waiting on the same two things:

1. **A public git repository.** A `PKGBUILD` needs a `source=` URL and a Flathub
   manifest needs a public source. There is no git remote configured.
2. **The reverse-DNS application id** (§15). It is still `scoria`.
   `scripts/check-boundaries.sh` enforces that the id agrees across
   `scoria-gui/src/lib.rs`, the desktop entry's basename, its `StartupWMClass`,
   its `Icon=`, and the icon files -- so the rename either lands everywhere or
   fails the build.

The AUR package needs only (1). Flathub needs both, plus screenshots.

---

## AUR

The channel that works the moment the repository is public, and the one that can
be tested natively on an Arch machine.

Two packages by convention: `scoria` from a release tag, and `scoria-git` from
`main`.

```bash
pkgname=scoria
pkgdesc='A library-centric music player'
arch=('x86_64')
license=('GPL-3.0-only')
depends=('alsa-lib' 'dbus')     # plus a Vulkan or GL driver, which any desktop has
makedepends=('cargo' 'git')     # rust >= 1.89, the MSRV
```

Build with `--locked` so the package resolves the versions the workspace pins
rather than whatever is current:

```bash
cargo build --release --locked --workspace
```

Install the same three files `just desktop-install` does -- that recipe in the
[`justfile`](../justfile) is the specification, the only difference being system
paths rather than `$XDG_DATA_HOME`:

| From | To |
|---|---|
| `target/release/scoria` | `/usr/bin/scoria` |
| `packaging/scoria.desktop` | `/usr/share/applications/` |
| `packaging/scoria.svg` | `/usr/share/icons/hicolor/scalable/apps/` |
| `packaging/scoria.png` | `/usr/share/icons/hicolor/256x256/apps/` |
| `LICENSE` | `/usr/share/licenses/scoria/` |

Check the result before submitting: `namcap` the built package, and
`desktop-file-validate` the installed entry.

---

## Flatpak

### Runtime

`org.freedesktop.Platform` with `org.freedesktop.Sdk.Extension.rust-stable`.
The MSRV is 1.89 (set by lofty), comfortably below what the extension carries.
Pin the runtime version in the manifest and bump it deliberately.

### Finish args

Reproduced from §15, which explains each:

```yaml
finish-args:
  - --device=dri
  - --socket=wayland
  - --socket=fallback-x11
  - --share=ipc
  - --socket=pulseaudio
  - --own-name=org.mpris.MediaPlayer2.scoria
  - --filesystem=xdg-music:ro
  # No --share=network. Not an omission: DESIGN.md 13.
```

Do not add `--filesystem=home`. The read-only music directory plus the portal is
the whole point, and a reviewer will ask.

### The offline build

Flathub builds with no network, so `Cargo.lock` has to become a sources list
before the manifest will build:

```bash
# from flathub/flatpak-builder-tools
python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
```

This regenerates whenever a dependency changes, so it belongs behind a `just`
recipe rather than a step someone has to remember. `cargo-sources.json` is large
and generated -- commit it, because Flathub needs it, but never hand-edit it.

Build and test locally before opening a Flathub PR:

```bash
flatpak-builder --user --install --force-clean build-dir <app-id>.yaml
flatpak run <app-id>
```

### Also needed by Flathub

- **An AppStream metainfo file**, `<app-id>.metainfo.xml`, with a summary,
  description, launchable, categories, a `<releases>` entry per version, and
  screenshots. Its filename is the app id, so it is blocked on that decision.
- **Screenshots.** Required, and they do not exist yet -- see the commented block
  in [`README.md`](../README.md), which names the three worth taking.
- The desktop entry and icons renamed to the app id, which the boundary check
  already enforces.

### Telling users about the library path

The one thing worth putting in the Flathub description, because it will
otherwise arrive as a bug report. A library outside `~/Music` needs:

```bash
flatpak override --user --filesystem=/path/to/music:ro <app-id>
```

§15 explains why this cannot be solved from inside the sandbox: the portal grant
is a FUSE path, and the library watcher needs real `inotify`.
