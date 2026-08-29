#!/usr/bin/env bash
# Structural enforcement of the CLAUDE.md invariants that a compiler cannot check.
# Run locally or from CI; exits non-zero on the first violation.
set -uo pipefail

status=0
fail() { printf '\033[31mFAIL\033[0m %s\n' "$1"; status=1; }
pass() { printf '\033[32mok\033[0m   %s\n' "$1"; }

cd "$(dirname "$0")/.."

# Invariant 1: scoria-core never depends on a UI crate.
# Checks the resolved dependency graph, not the manifest, so a UI crate arriving
# transitively through some innocent-looking dependency is caught too.
ui_deps=$(cargo tree -p scoria-core -e normal 2>/dev/null \
  | grep -oE '\b(iced|iced_core|iced_widget|iced_winit|ratatui|winit|softbuffer|crossterm)\b' \
  | sort -u)
if [ -n "$ui_deps" ]; then
  fail "scoria-core depends on a UI crate: $(echo "$ui_deps" | tr '\n' ' ')"
  echo "     CLAUDE.md invariant 1. If core needs something from the UI, the design is wrong."
else
  pass "scoria-core depends on no UI crate"
fi

# Invariant 3 / DESIGN.md 7.3: exactly one track table implementation.
#
# The first version of this check looked for `fn table`, which the real table
# does not define -- it exposes `view`. So it matched nothing and reported green
# from the day it was written. A check that cannot fail is worse than no check,
# because it is mistaken for coverage.
#
# The rule now: anything that looks like a table lives in widgets/table.rs.
# Anywhere else, a file named for tables or a function named for them is the
# start of a second implementation.
THE_TABLE='scoria-gui/src/widgets/table.rs'
if [ -d scoria-gui/src ]; then
  if [ ! -f "$THE_TABLE" ]; then
    pass "no track table yet"
  else
    rivals=$(
      find scoria-gui/src -name '*.rs' ! -path "$THE_TABLE" 2>/dev/null | sort | while read -r f; do
        case "$(basename "$f")" in
          *table*) echo "$f (named for tables)"; continue ;;
        esac
        # Tests are cut first: a test *about* the table is named for it and is
        # not a second one. `on_table` and friends are message handlers, not
        # implementations.
        if sed '/^#\[cfg(test)\]/,$d' "$f" \
          | grep -E '^[[:space:]]*(pub )?fn [a-z_]*table[a-z_]*[(<]' \
          | grep -qvE 'fn (on|handle|update)_'; then
          echo "$f (defines a table function)"
        fi
      done
    )
    if [ -n "$rivals" ]; then
      fail "more than one track table implementation:"
      echo "$rivals" | sed 's/^/       /'
      echo "     CLAUDE.md invariant 3. Layouts arrange the shared table; they do not define one."
    else
      pass "one track table, in widgets/table.rs"
    fi
  fi
fi

# DESIGN.md 11: iced Color never escapes the theme module.
if [ -d scoria-gui/src ]; then
  stray=$(grep -rlE '\biced::Color\b|\bColor::(from_rgb|WHITE|BLACK)' scoria-gui/src 2>/dev/null \
    | grep -v '^scoria-gui/src/theme/' | sort)
  if [ -n "$stray" ]; then
    fail "iced Color used outside scoria-gui/src/theme/:"
    echo "$stray" | sed 's/^/       /'
    echo "     Themes are expressed in our vocabulary; the iced layer converts at the boundary."
  else
    pass "no iced Color outside theme/"
  fi
fi

# Conventions: raw SQL belongs to the repository layer.
sql=$(grep -rlE '"[[:space:]]*(SELECT|INSERT|UPDATE|DELETE|CREATE)[[:space:]]' \
  scoria-core/src scoria-gui/src scoria-bin/src 2>/dev/null \
  | grep -v '^scoria-core/src/db/' | sort)
if [ -n "$sql" ]; then
  fail "raw SQL outside scoria-core/src/db/:"
  echo "$sql" | sed 's/^/       /'
  echo "     Database access goes through repository traits."
else
  pass "no raw SQL outside db/"
fi

# Panic policy: the workspace denies the constructs that can panic at runtime, but
# a local `allow` silently reopens the hole. Test modules are legitimately allowed
# to unwrap; production code is not. Test modules are conventionally last in the
# file, so anything before `mod tests` is treated as production code.
panic_lints='unwrap_used|expect_used|panic|todo|unimplemented|unreachable|indexing_slicing|string_slice|panic_in_result_fn'

# Whole modules declared `#[cfg(test)] mod foo;` are test-only, so an allow
# anywhere in foo.rs is legitimate. Collect their names first.
test_only_modules=$(
  find scoria-*/src -name '*.rs' 2>/dev/null \
    | xargs grep -h -A1 '^#\[cfg(test)\]$' 2>/dev/null \
    | grep -oE '^[[:space:]]*(pub )?mod [a-z_0-9]+;' \
    | grep -oE '[a-z_0-9]+;' | tr -d ';' | sort -u
)

escapes=$(
  find scoria-*/src -name '*.rs' 2>/dev/null | sort | while read -r f; do
    stem=$(basename "$f" .rs)
    if [ -n "$test_only_modules" ] && echo "$test_only_modules" | grep -qx "$stem"; then
      continue
    fi
    # Otherwise, test modules are conventionally last in the file, so anything
    # before `mod tests` is production code.
    boundary=$(grep -n '^[[:space:]]*mod tests' "$f" | head -1 | cut -d: -f1)
    [ -z "$boundary" ] && boundary=999999
    grep -nE "allow\((clippy::)?($panic_lints)" "$f" \
      | awk -F: -v b="$boundary" -v f="$f" '$1 < b { print f ":" $1 }'
  done
)
if [ -n "$escapes" ]; then
  fail "panic lint allowed outside a test module:"
  echo "$escapes" | sed 's/^/       /'
  echo "     The goal is no known way to panic at runtime. Handle the error instead,"
  echo "     or move the code under #[cfg(test)]."
else
  pass "no panic lints allowed outside test modules"
fi

# DESIGN.md 13: Scoria never talks to the internet.
#
# Checked against the dependency graph rather than against the code, because
# that is where it would arrive: nobody writes a socket by hand, they add a
# crate that already has one. D-Bus is deliberately not listed -- souvlaki talks
# to the desktop over a local socket (DESIGN.md 12), which is not the internet.
network_crates='^(reqwest|hyper|hyper-util|ureq|isahc|surf|attohttpc|curl|curl-sys|awc|tungstenite|tokio-tungstenite|rustls|native-tls|openssl|openssl-sys|hickory-resolver|hickory-proto|trust-dns-resolver)$'
if command -v cargo >/dev/null; then
  online=$(cargo tree --workspace -e normal 2>/dev/null \
    | grep -oE '[a-z0-9_-]+ v[0-9]' | sed 's/ v[0-9]//' | sort -u \
    | grep -E "$network_crates" || true)
  if [ -n "$online" ]; then
    fail "a network client is in the dependency graph: $(echo "$online" | tr '\n' ' ')"
    echo "     DESIGN.md 13. Scoria reads the library on disk and nothing else."
  else
    pass "nothing in the tree can reach the internet"
  fi
fi

exit $status
