#!/usr/bin/env bash
# Source after session.sh. stage_up/stage_down with cleanup trap.
# Disables Hyprland dim during capture and restores it after.

stage_down() {
  pkill -9 -f target/debug/strikehub 2>/dev/null || true
  pkill -9 wf-recorder 2>/dev/null || true
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1 || true
  # restore dim defaults
  hyprq keyword decoration:dim_inactive false >/dev/null 2>&1 || true
}

stage_up() {
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
  hyprq keyword monitor "$hl,1920x1080@60,1920x0,1" >/dev/null 2>&1
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1
  echo "$hl"
}

trap stage_down EXIT INT TERM
