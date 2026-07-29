#!/usr/bin/env bash
# Launch StrikeHub under the validated headless X11 env. Backgrounds it.
cd "$(dirname "$0")/../.." || exit 1   # repo root (demo/env -> repo)
nix develop --command bash -c '
  export XDG_RUNTIME_DIR="'"${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"'"
  export DISPLAY=:0 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export STRIKE48_API_URL=https://plg.strike48.test MATRIX_TLS_INSECURE=1 RUST_LOG=warn
  exec ./target/debug/strikehub
' > /tmp/strikehub-demo.log 2>&1 &
echo $!
