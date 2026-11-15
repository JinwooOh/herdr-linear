#!/usr/bin/env bash
# Launch or focus the Linear pane in the current tab; never toggle-close it.
set -uo pipefail
herdr_bin="${HERDR_BIN_PATH:-herdr}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
viewer_bin="$script_dir/../target/release/herdr-linear"
if [ -x "$viewer_bin" ]; then
  panes="$("$herdr_bin" pane list 2>/dev/null || true)"
  decision="$(printf '%s' "$panes" | "$viewer_bin" --launch-decision 2>/dev/null || echo OPEN)"
  case "$decision" in
    "FOCUS "*)
      # The Rust decision validates the ID before it reaches argv.
      if "$herdr_bin" plugin pane focus "${decision#FOCUS }"; then exit 0; fi
      # The pane may have closed after the list request; fall back to opening.
      ;;
  esac
fi
exec "$herdr_bin" plugin pane open --plugin herdr-linear --entrypoint issues --placement split --focus
