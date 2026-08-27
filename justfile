# Scoria -- task runner. `just` on its own lists everything.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Where the CLI points when a recipe needs a library. Override: just library=/path scan
library := env_var_or_default("SCORIA_LIBRARY", env_var("HOME") + "/Music")

_default:
    @just --list --unsorted

# ---------------------------------------------------------------- everyday --

# Debug build of the whole workspace.
build:
    cargo build --workspace

# Run the player. Extra arguments pass through: just run --help
run *ARGS:
    cargo run --bin scoria -- {{ARGS}}

# Run the tests. Optional filter: just test sort_chain
test *FILTER:
    cargo test --workspace {{FILTER}}

# Tests with output from passing tests shown too.
test-loud *FILTER:
    cargo test --workspace {{FILTER}} -- --nocapture

# Type-check without producing binaries. Fastest feedback loop.
watch:
    #!/usr/bin/env bash
    if ! command -v cargo-watch >/dev/null; then
        echo "needs cargo-watch: cargo install cargo-watch" >&2; exit 1
    fi
    cargo watch -x 'check --workspace --all-targets'

# ------------------------------------------------------------------- gates --

# Everything CI runs. Run this before committing.
check: fmt-check clippy test invariants
    @echo
    @echo "all gates passed"

# Apply formatting.
fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# The CLAUDE.md invariants the compiler cannot express.
invariants:
    ./scripts/check-boundaries.sh

# Build against the declared MSRV floor rather than the local toolchain.
msrv:
    #!/usr/bin/env bash
    v=$(grep -m1 '^rust-version' Cargo.toml | sed 's/[^0-9.]//g')
    echo "checking against Rust $v"
    if ! rustup toolchain list | grep -q "^$v"; then
        echo "installing toolchain $v"; rustup toolchain install "$v" --profile minimal
    fi
    cargo "+$v" check --workspace --all-targets

# --------------------------------------------------------------- releasing --

# Optimised build.
release:
    cargo build --workspace --release

# Install the binary into ~/.cargo/bin.
install:
    cargo install --path scoria-cli --locked

# --------------------------------------------------------------- inspecting --

# The architecture of the query engine rests on these numbers -- see the module
# docs on LibraryIndex. Re-run after touching the engine, the comparator or the
# index. Takes a track count; defaults to the six figures §14 designs for.

# Benchmark the query engine at scale.
bench-query count="100000":
    cargo run --release -p scoria-core --example query_scale -- {{count}}

# Advisory only: does not fail, and most hits are fine. See the
# arithmetic_side_effects note in Cargo.toml for what is deliberately not denied.

# Report constructs that could panic, beyond the set already denied.
panics:
    -cargo clippy --workspace --all-targets -- \
        -W clippy::arithmetic_side_effects \
        -W clippy::integer_division \
        -W clippy::modulo_arithmetic \
        -W clippy::cast_possible_truncation \
        -W clippy::fallible_impl_from \
        -W clippy::missing_panics_doc \
        -W clippy::rc_buffer

# Open the API docs for the workspace.
doc:
    cargo doc --workspace --no-deps --open

# What scoria-core actually pulls in. Should contain no UI crate.
tree-core:
    cargo tree -p scoria-core -e normal

# Full dependency graph.
tree:
    cargo tree --workspace -e normal

# Known security advisories.
audit:
    #!/usr/bin/env bash
    if ! command -v cargo-audit >/dev/null; then
        echo "needs cargo-audit: cargo install cargo-audit" >&2; exit 1
    fi
    cargo audit

# Pins here are deliberate. Read the Stack section of CLAUDE.md before bumping
# anything -- symphonia in particular must track whatever rodio resolves.

# Dependencies with newer releases available.
outdated:
    #!/usr/bin/env bash
    if ! command -v cargo-outdated >/dev/null; then
        echo "needs cargo-outdated: cargo install cargo-outdated" >&2; exit 1
    fi
    cargo outdated --workspace --root-deps-only

# ----------------------------------------------------------------- library --
# These become useful at M6, when scoria-cli can drive the core.

# Scan a library root into the database.
scan dir=library:
    cargo run --bin scoria -- scan {{dir}}

# Scoria reads gain from tags and never computes it, so a low number here is
# fixed by tagging the library: `rsgain easy <dir>`.

# Report what fraction of the library carries ReplayGain tags.
gain-coverage:
    #!/usr/bin/env bash
    if ! command -v metaflac >/dev/null; then
        echo "needs metaflac (flac package)" >&2; exit 1
    fi
    total=0; tagged=0
    while IFS= read -r f; do
        total=$((total + 1))
        if metaflac --export-tags-to=- "$f" 2>/dev/null | grep -q '^REPLAYGAIN_TRACK_GAIN='; then
            tagged=$((tagged + 1))
        fi
    done < <(find "{{library}}" -iname '*.flac')
    if [ "$total" -eq 0 ]; then echo "no FLAC files under {{library}}"; exit 0; fi
    echo "$tagged/$total FLAC files carry ReplayGain tags ($((tagged * 100 / total))%)"

# ------------------------------------------------------------ housekeeping --

clean:
    cargo clean

# Remove build output and the thumbnail cache.
clean-all: clean
    rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/scoria"
