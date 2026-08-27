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
if [ -d scoria-gui/src ]; then
  tables=$(find scoria-gui/src -name '*.rs' -exec grep -lE 'fn +(track_)?table\b' {} + 2>/dev/null | sort)
  count=$(printf '%s' "$tables" | grep -c . || true)
  if [ "$count" -gt 1 ]; then
    fail "more than one track table implementation:"
    echo "$tables" | sed 's/^/       /'
    echo "     CLAUDE.md invariant 3. Layouts arrange the shared table; they do not define one."
  else
    pass "at most one track table implementation"
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
  scoria-core/src scoria-gui/src scoria-cli/src 2>/dev/null \
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
escapes=$(
  find scoria-*/src -name '*.rs' 2>/dev/null | sort | while read -r f; do
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

exit $status
