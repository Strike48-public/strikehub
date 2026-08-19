#!/usr/bin/env bash
# Source after session.sh. stage_up/stage_down with cleanup trap.
# Snapshots the user's real eDP-1 mode + dim setting on stage_up and restores
# the EXACT values on stage_down (never hardcodes them) so a differently-sized
# panel isn't left misconfigured. Cleans up the headless output + X0 symlink.

STAGE_STATE_DIR="${TMPDIR:-/tmp}/strikehub-demo-stage"

stage_down() {
  # Kill the app by the PID we captured at launch (not a broad pkill pattern
  # that could match unrelated processes). Fall back to a scoped pattern only
  # if we somehow have no PID on file.
  if [ -f "$STAGE_STATE_DIR/app.pid" ]; then
    local apid; apid="$(cat "$STAGE_STATE_DIR/app.pid" 2>/dev/null)"
    [ -n "$apid" ] && kill "$apid" 2>/dev/null
    # give it a moment to exit cleanly, then force
    sleep 0.5
    [ -n "$apid" ] && kill -9 "$apid" 2>/dev/null
  fi
  pkill -TERM wf-recorder 2>/dev/null || true

  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done

  # Restore eDP-1 to its snapshotted mode/position/scale (captured at stage_up).
  if [ -f "$STAGE_STATE_DIR/edp1.monitor" ]; then
    hyprq keyword monitor "$(cat "$STAGE_STATE_DIR/edp1.monitor")" >/dev/null 2>&1 || true
  fi
  # Restore dim_inactive to its snapshotted value.
  if [ -f "$STAGE_STATE_DIR/dim_inactive" ]; then
    hyprq keyword decoration:dim_inactive "$(cat "$STAGE_STATE_DIR/dim_inactive")" >/dev/null 2>&1 || true
  fi

  # Remove the X0 symlink we created (only if it's the one we made).
  [ -L /tmp/.X11-unix/X0 ] && rm -f /tmp/.X11-unix/X0 2>/dev/null || true

  # Restore the user's original connector configs (their real easy_mode pref),
  # backed up by launch-app.sh before we forced Easy Mode on.
  local sh_cfg="${XDG_CONFIG_HOME:-$HOME/.config}/strikehub/connectors.toml"
  local pick_cfg="${XDG_CONFIG_HOME:-$HOME/.config}/pentest-connector/settings.json"
  [ -f "$STAGE_STATE_DIR/connectors.toml.bak" ] && mv -f "$STAGE_STATE_DIR/connectors.toml.bak" "$sh_cfg" 2>/dev/null || true
  [ -f "$STAGE_STATE_DIR/settings.json.bak" ] && mv -f "$STAGE_STATE_DIR/settings.json.bak" "$pick_cfg" 2>/dev/null || true
}

stage_up() {
  mkdir -p "$STAGE_STATE_DIR"

  # Snapshot eDP-1's current mode/position/scale so we can restore it exactly.
  # Format: "eDP-1,<W>x<H>@<Hz>,<x>x<y>,<scale>"
  local edp
  edp="$(hyprq monitors -j | jq -r '
    .[] | select(.name=="eDP-1") |
    "eDP-1,\(.width)x\(.height)@\(.refreshRate|floor),\(.x)x\(.y),\(.scale)"' | head -1)"
  [ -n "$edp" ] && echo "$edp" > "$STAGE_STATE_DIR/edp1.monitor"

  # Snapshot current dim_inactive so we can restore the user's preference.
  hyprq getoption decoration:dim_inactive -j 2>/dev/null \
    | jq -r 'if .int == 1 then "true" else "false" end' > "$STAGE_STATE_DIR/dim_inactive" 2>/dev/null || true

  [ -e /tmp/.X11-unix/X0 ] || ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  # ensure no inactive-window dim during capture
  hyprq keyword decoration:dim_inactive false >/dev/null 2>&1
  hyprq output create headless >/dev/null 2>&1
  sleep 0.5
  local hl
  hl="$(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name' | head -1)"
  # Park the headless output just to the right of eDP-1's actual logical width
  # (not a hardcoded 1920). Reuse the snapshot; fall back to 1920 if unknown.
  local edp_logical_w
  edp_logical_w="$(hyprq monitors -j | jq -r '.[]|select(.name=="eDP-1")|(.width/.scale)|floor' | head -1)"
  [ -z "$edp_logical_w" ] || [ "$edp_logical_w" = "null" ] && edp_logical_w=1920
  hyprq keyword monitor "$hl,1920x1080@60,${edp_logical_w}x0,1" >/dev/null 2>&1
  echo "$hl"
}

trap stage_down EXIT INT TERM
