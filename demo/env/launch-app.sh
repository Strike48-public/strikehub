#!/usr/bin/env bash
# Launch StrikeHub under the validated headless X11 env. Backgrounds it.

# Deterministic start state: the demo's toggle scenes flip Easy Mode OFF and the
# apps PERSIST that to config, so a prior run leaves Easy Mode off and the next
# run would start already-expert (or the toggle would flip the wrong way). Force
# both Easy Modes ON before every launch so the flow always starts in Easy Mode.
SH_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/strikehub/connectors.toml"
PICK_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/pentest-connector/settings.json"
STATE_DIR="${TMPDIR:-/tmp}/strikehub-demo-stage"
mkdir -p "$STATE_DIR"
# Back up the user's configs BEFORE we mutate them, so a later restore can put
# their real easy_mode preference back (the demo scenes toggle it off and the
# app persists that, otherwise permanently overwriting a user setting).
[ -f "$SH_CFG" ] && [ ! -f "$STATE_DIR/connectors.toml.bak" ] && cp "$SH_CFG" "$STATE_DIR/connectors.toml.bak"
[ -f "$PICK_CFG" ] && [ ! -f "$STATE_DIR/settings.json.bak" ] && cp "$PICK_CFG" "$STATE_DIR/settings.json.bak"
# Force Easy Mode ON for a deterministic start. Allow leading whitespace so an
# indented TOML key still matches (the old ^-anchored regex silently missed it).
[ -f "$SH_CFG" ] && sed -i -E 's/^[[:space:]]*easy_mode[[:space:]]*=.*/easy_mode = true/' "$SH_CFG"
[ -f "$PICK_CFG" ] && sed -i -E 's/"easy_mode"[[:space:]]*:[[:space:]]*(true|false)/"easy_mode": true/' "$PICK_CFG"

cd "$(dirname "$0")/../.." || exit 1   # repo root (demo/env -> repo)
nix develop --command bash -c '
  export XDG_RUNTIME_DIR="'"${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"'"
  export DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1 RUST_LOG=warn
  exec ./target/debug/strikehub
' > /tmp/strikehub-demo.log 2>&1 &
APP_PID=$!
# Record the PID so stage_down kills exactly this process (not a broad pkill).
echo "$APP_PID" > "$STATE_DIR/app.pid"
echo "$APP_PID"
