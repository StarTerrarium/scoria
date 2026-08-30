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

# DESIGN.md 7.3: every column exists in the panel that turns it on.
#
# The compiler forces a new `Column` variant to gain a heading, a name, a value
# and a sort field -- every match over it is exhaustive. It does not force the
# variant into `ALL`, which is a hand-written array, and a column missing from
# there is a column nobody can switch on. That happened once: the play-count
# column shipped with storage, a sort field and a commit message describing it,
# and no way to see it.
COLUMNS='scoria-gui/src/widgets/columns.rs'
if [ -f "$COLUMNS" ]; then
  declared=$(sed -n '/^pub enum Column {/,/^}/p' "$COLUMNS" \
    | grep -oE '^    [A-Z][A-Za-z]*,$' | tr -d ' ,' | sort -u)
  offered=$(sed -n '/pub const ALL:/,/^    \];$/p' "$COLUMNS" \
    | grep -oE 'Self::[A-Z][A-Za-z]*' | sed 's/Self:://' | sort -u)
  unoffered=$(comm -23 <(echo "$declared") <(echo "$offered"))
  if [ -n "$unoffered" ]; then
    fail "column not offered in the panel: $(echo "$unoffered" | tr '\n' ' ')"
    echo "     DESIGN.md 7.3. Add it to Column::ALL, or it cannot be switched on."
  else
    pass "every column is offered in the panel"
  fi
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

# Invariant 3 / DESIGN.md 11: a widget helper nothing calls is a feature
# nobody sees.
#
# `Column::value_for` shipped with a marker constant, a commit message and
# 26 passing tests, and the table never called it -- so the now-playing
# glyph DESIGN.md 11 requires had never once rendered. The tests passed
# because they called the helper directly. Nothing was watching the seam
# between the helper and the row that draws it.
#
# So: every `pub fn` under widgets/ must be named somewhere outside its own
# definition and outside a test module. Test-only helpers belong in the test
# module that uses them.
#
# A tripwire rather than a proof: it matches on the bare name, so a helper
# called something that appears in prose elsewhere -- `view`, `value` -- is
# mentioned often enough to pass whether or not anyone calls it. It catches
# the distinctive names, which is where this mistake actually happens.
if [ -d scoria-gui/src/widgets ]; then
  # Production code only: test modules are conventionally last in the file.
  production=$(mktemp)
  find scoria-gui/src scoria-bin/src -name '*.rs' 2>/dev/null | sort | while read -r f; do
    sed '/^#\[cfg(test)\]/,$d' "$f"
  done > "$production"

  uncalled=$(
    find scoria-gui/src/widgets -name '*.rs' 2>/dev/null | sort | while read -r f; do
      sed '/^#\[cfg(test)\]/,$d' "$f" \
        | grep -oE '^[[:space:]]*pub (const )?fn [a-z_][a-z_0-9]*' \
        | grep -oE '[a-z_][a-z_0-9]*$' | while read -r name; do
          # The definition itself is one mention; anything else is a caller.
          n=$(grep -cwF "$name" "$production")
          [ "$n" -le 1 ] && echo "$f: $name"
        done
    done
  )
  rm -f "$production"

  if [ -n "$uncalled" ]; then
    fail "a widget function nothing calls:"
    echo "$uncalled" | sed 's/^/       /'
    echo "     A helper with no caller is a feature that never renders."
  else
    pass "every widget function has a caller"
  fi
fi

# Invariant 2 / DESIGN.md 3.2: a command nothing sends is not part of the seam.
#
# The same mistake as the widget check above, one layer down. `command.rs` is
# the first file anyone reads to understand this architecture, so a variant
# listed there is a claim that some frontend sends it. Four of them were not:
# `Play` bypassed the play-behaviour setting, `Rescan` dispatched to a blocking
# foreground scan, `SetRoot` duplicated the method the GUI actually calls, and
# `Query` answered with an event carrying too little to render.
#
# Dead is the mild version of the problem. The real one is that each was a trap
# for the next frontend, which would have had no way to know the path was never
# taken.
#
# So: every `Command` variant is constructed somewhere other than the enum
# itself. Match arms do not count -- handling a command is not sending one --
# so lines where the variant sits left of a `=>` are dropped. That filter has
# to be `Command::V[^=]*=>` and not a bare `=>`, or genuine construction sites
# like `Message::X => Command::Seek(p)` are dropped with the patterns.
variants=$(sed -n '/^pub enum Command/,/^}/p' scoria-core/src/command.rs \
  | grep -oE '^    [A-Z][A-Za-z]*' | tr -d ' ')

if [ -z "$variants" ]; then
  fail "could not read the Command enum; this check is not running"
else
  unsent=$(
    for v in $variants; do
      n=$(grep -rn "Command::$v\b" scoria-core/src scoria-gui/src scoria-bin/src 2>/dev/null \
        | grep -v '^scoria-core/src/command.rs:' \
        | grep -vE "Command::$v\b[^=]*=>" \
        | grep -vE ':[[:space:]]*(//|\*)' \
        | wc -l)
      [ "$n" -eq 0 ] && echo "Command::$v"
    done
  )

  if [ -n "$unsent" ]; then
    fail "a command nothing sends:"
    echo "$unsent" | sed 's/^/       /'
    echo "     A variant no frontend constructs is a seam that was never tested."
  else
    pass "every command has a sender"
  fi
fi

# DESIGN.md 15: the desktop identity is one string in several places.
#
# `application_id` is what a Wayland compositor matches against the desktop
# entry's basename to find the icon, and `Icon=` is what the entry points at.
# Get them out of step and nothing fails: the build is clean, the tests pass,
# and the window shows the compositor's placeholder icon instead of Scoria's.
# `scoria-gui/src/lib.rs` already documents that failure; nothing was checking
# for it.
#
# This matters now because the id has to change. Every store format wants a
# reverse-DNS one, and that rename has to land in all of these at once.
appid=$(grep -oE 'application_id: "[^"]+"' scoria-gui/src/lib.rs 2>/dev/null \
  | head -1 | sed 's/.*"\(.*\)"/\1/')

if [ -z "$appid" ]; then
  fail "could not read application_id from scoria-gui/src/lib.rs"
else
  mismatch=""
  desktop="packaging/$appid.desktop"

  if [ ! -f "$desktop" ]; then
    mismatch="$mismatch\n  no packaging/$appid.desktop for application_id \"$appid\""
  else
    wmclass=$(grep -oE '^StartupWMClass=.*' "$desktop" | head -1 | cut -d= -f2-)
    iconname=$(grep -oE '^Icon=.*' "$desktop" | head -1 | cut -d= -f2-)
    [ "$wmclass" = "$appid" ] || \
      mismatch="$mismatch\n  StartupWMClass=$wmclass, but application_id is \"$appid\""
    [ "$iconname" = "$appid" ] || \
      mismatch="$mismatch\n  Icon=$iconname, but application_id is \"$appid\""
  fi

  # The names `Icon=` resolves to once installed, and the one compiled in.
  for ext in svg png; do
    [ -f "packaging/$appid.$ext" ] || \
      mismatch="$mismatch\n  no packaging/$appid.$ext for Icon=$appid"
  done
  grep -q "packaging/$appid.png" scoria-gui/src/lib.rs || \
    mismatch="$mismatch\n  the window icon is not include_bytes!(\".../$appid.png\")"

  # AppStream carries the id a fourth time. GNOME Software and KDE Discover
  # both read it, and Flathub will not take a submission without it, so a
  # metainfo file naming a different id is an application that installs and
  # then cannot be found in a store.
  metainfo="packaging/$appid.metainfo.xml"
  if [ ! -f "$metainfo" ]; then
    mismatch="$mismatch\n  no $metainfo"
  else
    mid=$(grep -oE '<id>[^<]+</id>' "$metainfo" | head -1 | sed 's|</\?id>||g')
    launch=$(grep -oE '<launchable[^>]*>[^<]+</launchable>' "$metainfo" \
      | head -1 | sed 's|.*>\(.*\)</launchable>|\1|')
    [ "$mid" = "$appid" ] || \
      mismatch="$mismatch\n  the metainfo <id> is $mid, but application_id is \"$appid\""
    [ "$launch" = "$appid.desktop" ] || \
      mismatch="$mismatch\n  the metainfo <launchable> is $launch, not $appid.desktop"
  fi

  if [ -n "$mismatch" ]; then
    fail "the desktop identity disagrees with itself:"
    printf '%b\n' "$mismatch" | sed '/^[[:space:]]*$/d; s/^/     /'
    echo "     A window whose id does not match its entry shows a placeholder icon."
  else
    pass "one desktop identity, \"$appid\", everywhere"
  fi
fi

# No real home directory in the source.
#
# Test fixtures and doc examples need absolute paths, and the obvious one to
# type is your own. A developer's real home path reached three files that way --
# error.rs, path.rs and tests/storage.rs -- and sat in 103 commits before
# anything was published. It leaks a login name, which is half of a credential
# guess, and it is free to fix before a push and a history rewrite afterwards.
#
# This comment deliberately does not spell out the offending path: the check
# scans scripts/ too, so naming it here would make the check fail on itself.
#
# Placeholders only. `someone` is what picker.rs and welcome.rs already used;
# `user` is what config.rs's documented example uses.
homes=$(git grep -hoIE '/home/[a-z_][a-z0-9_-]*' -- '*.rs' '*.md' '*.toml' 'justfile' 'scripts/*' 2>/dev/null \
  | sort -u | grep -vE '^/home/(someone|user)$')

if [ -n "$homes" ]; then
  fail "a real home directory in the source:"
  echo "$homes" | sed 's/^/       /'
  echo "     Use /home/someone. A developer's own path leaks a login name."
else
  pass "no real home directory in the source"
fi

exit $status
