#!/usr/bin/env bash
# get-pii-names.sh - Pull the shared PII name list into .pii-names.local.
#
# The list must never live in this public repo, so it is shared out-of-band:
# a sops-encrypted YAML file (a `names:` list) in Strike48/dev-secrets,
# encrypted to every developer's age key. Fetched with the gh CLI and
# decrypted locally with sops, so you need repo access and your age key.
# To change the list, edit it in dev-secrets with `sops`, then re-run this.
#
# Configure the source with env vars or flags:
#   PII_NAMES_REPO   OWNER/REPO holding the list (default: Strike48/dev-secrets)
#   PII_NAMES_PATH   path to the file (default: secrets/pii/names.sops.yaml)
#   PII_NAMES_REF    optional branch/tag/SHA (default: repo default branch)
#
# Usage:
#   scripts/get-pii-names.sh
#   scripts/get-pii-names.sh --ref my-branch
#   just pii-pull

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPO_ROOT
readonly DEST="${REPO_ROOT}/.pii-names.local"

# Source: the sops file in dev-secrets. Override with env or flags.
SRC_REPO="${PII_NAMES_REPO:-Strike48/dev-secrets}"
SRC_PATH="${PII_NAMES_PATH:-secrets/pii/names.sops.yaml}"
SRC_REF="${PII_NAMES_REF:-}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo) SRC_REPO="${2:?--repo needs OWNER/REPO}"; shift 2 ;;
        --path) SRC_PATH="${2:?--path needs a value}"; shift 2 ;;
        --ref)  SRC_REF="${2:?--ref needs a value}"; shift 2 ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; exit 2 ;;
    esac
done

for tool in gh sops jq; do
    command -v "$tool" >/dev/null 2>&1 || { echo "ERROR: $tool not found." >&2; exit 2; }
done

api="repos/${SRC_REPO}/contents/${SRC_PATH}"
[[ -n "$SRC_REF" ]] && api="${api}?ref=${SRC_REF}"

# Owner-only: the decrypted list sits here until it is moved into place.
umask 077
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
enc="${work}/names.sops.yaml"
tmp="${work}/names.txt"

if ! gh api "$api" -H "Accept: application/vnd.github.raw" > "$enc" 2>/dev/null; then
    echo "ERROR: could not fetch ${SRC_PATH} from ${SRC_REPO}." >&2
    echo "Check the repo/path and that you have access." >&2
    exit 2
fi

# sops errors name the file and key, never a value; jq gets only decrypted JSON.
if ! sops -d --output-type json "$enc" | jq -er '.names[] | select(type == "string")' > "$tmp"; then
    echo "ERROR: could not decrypt ${SRC_PATH} or read its names: list." >&2
    echo "Check that your age key is a recipient (SOPS_AGE_KEY_FILE or ~/.config/sops/age/keys.txt)." >&2
    exit 2
fi

# Refuse to overwrite the local list with an empty/whitespace-only fetch.
if ! grep -qE '[^[:space:]]' "$tmp"; then
    echo "ERROR: fetched list from ${SRC_REPO}/${SRC_PATH} is empty; not overwriting ${DEST}." >&2
    exit 2
fi

mv "$tmp" "$DEST"
names="$(grep -vcE '^[[:space:]]*(#|$)' "$DEST" || true)"
echo "Wrote ${DEST} from ${SRC_REPO}/${SRC_PATH} (${names} name(s)). Names not printed."
echo "Maintainers: run 'just pii-sync' to push these to the CI secret."
