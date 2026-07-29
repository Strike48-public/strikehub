#!/usr/bin/env bash
# Source me. Resolves the live Hyprland session + ydotool socket.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-/run/ydotoold/socket}"

# Find the Hyprland instance signature whose socket actually responds.
_resolve_hypr_sig() {
  local d sig
  for d in "$XDG_RUNTIME_DIR"/hypr/*/; do
    sig="$(basename "$d")"
    if HYPRLAND_INSTANCE_SIGNATURE="$sig" hyprctl version >/dev/null 2>&1; then
      echo "$sig"; return 0
    fi
  done
  return 1
}

HYPR_SIG="$(_resolve_hypr_sig)" || { echo "ERROR: no live Hyprland instance" >&2; return 1 2>/dev/null || exit 1; }
export HYPRLAND_INSTANCE_SIGNATURE="$HYPR_SIG"
export HYPR_SIG

# hyprctl wrapper
hyprq() { hyprctl "$@"; }

# ydotool wrapper: runs through sg so the ydotool group is active without re-login.
ydo() { sg ydotool -c "YDOTOOL_SOCKET=$YDOTOOL_SOCKET ydotool $*"; }
