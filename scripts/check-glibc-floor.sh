#!/usr/bin/env bash
# Fail if any ELF file in an extracted AppImage needs a newer glibc than the
# supported floor. glibc is not bundled, so the floor decides which distros
# can run StrikeHub at all; a runner image upgrade raised it silently once.
#
# Usage: check-glibc-floor.sh <extracted-appimage-dir> [floor]
set -euo pipefail

ROOT=${1:?usage: check-glibc-floor.sh <extracted-appimage-dir> [floor]}
FLOOR=${2:-2.35}

BAD=0
CHECKED=0
while IFS= read -r -d '' f; do
    file -b "$f" | grep -q '^ELF' || continue
    CHECKED=$((CHECKED + 1))
    need=$(objdump -T "$f" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1 || true)
    [ -n "$need" ] || continue
    if [ "$(printf '%s\n%s\n' "$FLOOR" "$need" | sort -V | tail -1)" != "$FLOOR" ]; then
        echo "needs glibc $need (floor $FLOOR): ${f#"$ROOT"/}"
        BAD=1
    fi
done < <(find "$ROOT" -type f -print0)

[ "$CHECKED" -gt 0 ] || { echo "no ELF files found under $ROOT"; exit 1; }
[ "$BAD" -eq 0 ] || { echo "AppImage requires a newer glibc than the supported floor $FLOOR"; exit 1; }
echo "all $CHECKED ELF files need glibc <= $FLOOR"
