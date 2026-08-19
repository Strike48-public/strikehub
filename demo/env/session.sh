#!/usr/bin/env bash
# Source me. Resolves the live Hyprland session + system ydotool socket + grim helper.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-/run/ydotoold/socket}"

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
export HYPRLAND_INSTANCE_SIGNATURE="$HYPR_SIG" HYPR_SIG

# Resolve the real Hyprland wayland display (grim needs it).
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}"

hyprq() { hyprctl "$@"; }
# grim the headless output: shot <headless-name> <out.png>
shot() { WAYLAND_DISPLAY="$WAYLAND_DISPLAY" grim -o "$1" "$2"; }
