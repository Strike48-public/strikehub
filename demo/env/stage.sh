#!/usr/bin/env bash
# Source after session.sh. Provides stage_up / stage_down with cleanup trap.

stage_down() {
  pkill -9 -f target/debug/strikehub 2>/dev/null || true
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1 || true
}

stage_up() {
  # X socket fix for global_hotkey / X11 backend
  [ -e /tmp/.X11-unix/X0 ] || ln -sf /tmp/.X11-unix/X0_ /tmp/.X11-unix/X0
  # clean any leftovers, then create fresh
  local h
  for h in $(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name'); do
    hyprq output remove "$h" >/dev/null 2>&1 || true
  done
  hyprq output create headless >/dev/null 2>&1
  sleep 0.5
  local hl
  hl="$(hyprq monitors -j | jq -r '.[]|select(.name|startswith("HEADLESS"))|.name' | head -1)"
  hyprq keyword monitor "$hl,1920x1080@60,5000x0,1" >/dev/null 2>&1
  hyprq keyword monitor "eDP-1,3840x2160@60,0x0,2" >/dev/null 2>&1
  echo "$hl"
}

trap stage_down EXIT INT TERM
