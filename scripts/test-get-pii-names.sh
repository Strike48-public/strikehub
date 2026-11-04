#!/usr/bin/env bash
# test-get-pii-names.sh - Regression tests for scripts/get-pii-names.sh.
#
# Runs a copy of the script inside a temp tree (so it never touches your real
# .pii-names.local) with stub gh and sops on PATH. Covers the fetch + decrypt
# path, the refusal paths (fetch failure, decrypt failure, empty list), the
# --ref passthrough, the owner-only file mode, and that no name is printed.
#
# Needs jq (as the script does). Exit 0 if all pass, 1 otherwise.
# ok/no always return 0, so `cond && ok || no` cannot run both arms.
# shellcheck disable=SC2015

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SRC="${SCRIPT_DIR}/get-pii-names.sh"

pass=0
fail=0
ok() { echo "  ok   $1"; pass=$((pass + 1)); }
no() { echo "  FAIL $1" >&2; fail=$((fail + 1)); }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/repo/scripts" "$T/bin"
cp "$SRC" "$T/repo/scripts/get-pii-names.sh"
readonly DEST="$T/repo/.pii-names.local"

# gh: records its argv, prints a fake ciphertext, exits $GH_RC.
cat >"$T/bin/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >"$T/gh.argv"
echo "names: [ENC[stub]]"
exit "\${GH_RC:-0}"
EOF
# sops: prints \$SOPS_JSON as the decrypted document, exits \$SOPS_RC.
cat >"$T/bin/sops" <<'EOF'
#!/usr/bin/env bash
[[ "${SOPS_RC:-0}" == 0 ]] || { echo "sops: no matching key" >&2; exit "$SOPS_RC"; }
printf '%s' "${SOPS_JSON:-}"
EOF
chmod +x "$T/bin/gh" "$T/bin/sops"

# run [ENV=VAL...] [-- args]: runs the copy; output in $T/out, exit in $rc.
run() {
    local envs=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do envs+=("$1"); shift; done
    [[ "${1:-}" == "--" ]] && shift
    rc=0
    env PATH="$T/bin:$PATH" "${envs[@]}" bash "$T/repo/scripts/get-pii-names.sh" "$@" >"$T/out" 2>&1 || rc=$?
}

readonly GOOD='{"names":["Zyx Planted One","Qvw Planted Two"]}'

run SOPS_JSON="$GOOD"
if [[ $rc == 0 && "$(cat "$DEST")" == $'Zyx Planted One\nQvw Planted Two' ]]; then
    ok "fetch + decrypt writes one name per line"
else
    no "fetch + decrypt writes one name per line (rc=$rc)"
fi
grep -q '(2 name(s))' "$T/out" && ok "reports the count" || no "reports the count"
grep -qE 'Zyx|Qvw' "$T/out" && no "a name was printed" || ok "no name is printed"
[[ "$(stat -f %Lp "$DEST" 2>/dev/null || stat -c %a "$DEST")" == 600 ]] \
    && ok "the list is owner-only (600)" || no "the list is owner-only (600)"
grep -q '^api repos/Strike48/dev-secrets/contents/secrets/pii/names.sops.yaml ' "$T/gh.argv" \
    && ok "defaults to the dev-secrets file" || no "defaults to the dev-secrets file"

run SOPS_JSON="$GOOD" -- --ref my-branch
grep -q 'names.sops.yaml?ref=my-branch ' "$T/gh.argv" && ok "--ref reaches the API path" || no "--ref reaches the API path"

printf 'keep me\n' >"$DEST"
run GH_RC=1 SOPS_JSON="$GOOD"
[[ $rc == 2 && "$(cat "$DEST")" == "keep me" ]] && ok "fetch failure: exit 2, list untouched" \
    || no "fetch failure: exit 2, list untouched (rc=$rc)"

run SOPS_RC=1
[[ $rc == 2 && "$(cat "$DEST")" == "keep me" ]] && grep -q 'could not decrypt' "$T/out" \
    && ok "decrypt failure: exit 2, list untouched" || no "decrypt failure: exit 2, list untouched (rc=$rc)"

run SOPS_JSON='{"names":[]}'
[[ $rc == 2 && "$(cat "$DEST")" == "keep me" ]] && ok "empty list: exit 2, list untouched" \
    || no "empty list: exit 2, list untouched (rc=$rc)"

run SOPS_JSON='{"other":["x"]}'
[[ $rc == 2 && "$(cat "$DEST")" == "keep me" ]] && ok "no names: key: exit 2, list untouched" \
    || no "no names: key: exit 2, list untouched (rc=$rc)"

echo "$pass passed, $fail failed"
[[ $fail == 0 ]]
