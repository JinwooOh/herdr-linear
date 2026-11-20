#!/usr/bin/env bash
# Launch the Linear pane in its own tab, or reach an existing one in this workspace; never close it.
set -uo pipefail
herdr_bin="${HERDR_BIN_PATH:-herdr}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
viewer_bin="$script_dir/../target/release/herdr-linear"
open_tab() {
  exec "$herdr_bin" plugin pane open --plugin herdr-linear --entrypoint issues --placement tab --focus
}
if [ -x "$viewer_bin" ]; then
  panes="$("$herdr_bin" pane list 2>/dev/null || true)"
  decision="$(printf '%s' "$panes" | "$viewer_bin" --launch-decision-tab 2>/dev/null || echo OPEN)"
  # The Rust decision validates IDs before they reach argv. Either target may have closed
  # after the list request; fall back to opening.
  case "$decision" in
    "FOCUS "*)
      if "$herdr_bin" plugin pane focus "${decision#FOCUS }"; then exit 0; fi
      ;;
    "SWITCHTAB "*)
      if "$herdr_bin" tab focus "${decision#SWITCHTAB }"; then exit 0; fi
      ;;
  esac
fi
open_tab
