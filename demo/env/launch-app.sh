#!/usr/bin/env bash
# Launch StrikeHub under the validated headless X11 env. Backgrounds it.

# Deterministic start state: the demo's toggle scenes flip Easy Mode OFF and the
# apps PERSIST that to config, so a prior run leaves Easy Mode off and the next
# run would start already-expert (or the toggle would flip the wrong way). Force
# both Easy Modes ON before every launch so the flow always starts in Easy Mode.
SH_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/strikehub/connectors.toml"
PICK_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/pentest-connector/settings.json"
[ -f "$SH_CFG" ] && sed -i -E 's/^easy_mode[[:space:]]*=.*/easy_mode = true/' "$SH_CFG"
[ -f "$PICK_CFG" ] && sed -i -E 's/"easy_mode"[[:space:]]*:[[:space:]]*(true|false)/"easy_mode": true/' "$PICK_CFG"

cd "$(dirname "$0")/../.." || exit 1   # repo root (demo/env -> repo)
nix develop --command bash -c '
  export XDG_RUNTIME_DIR="'"${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"'"
  export DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1 RUST_LOG=warn
  exec ./target/debug/strikehub
' > /tmp/strikehub-demo.log 2>&1 &
echo $!
