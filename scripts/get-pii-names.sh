#!/usr/bin/env bash
# get-pii-names.sh - Pull the shared PII name list into .pii-names.local.
#
# The list must never live in this public repo, so it is shared out-of-band.
# Interim source: a file in a PRIVATE repo, fetched with the gh CLI. The infra
# team may switch this to a secrets manager later - only this script changes.
#
# Configure the source with env vars or flags:
#   PII_NAMES_REPO   OWNER/REPO of the PRIVATE repo holding the list (required)
#   PII_NAMES_PATH   path to the list file in that repo (default: pii-names.txt)
#   PII_NAMES_REF    optional branch/tag/SHA (default: repo default branch)
#
# Usage:
#   scripts/get-pii-names.sh
#   scripts/get-pii-names.sh --repo Strike48/some-private-repo --path pii-names.txt
#   PII_NAMES_REPO=Strike48/some-private-repo just pii-pull

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPO_ROOT
readonly DEST="${REPO_ROOT}/.pii-names.local"

# Source (a PRIVATE repo). Override with env or flags. Leave repo empty so the
# script fails loud rather than guessing a wrong repo.
SRC_REPO="${PII_NAMES_REPO:-}"
SRC_PATH="${PII_NAMES_PATH:-pii-names.txt}"
SRC_REF="${PII_NAMES_REF:-}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo) SRC_REPO="${2:?--repo needs OWNER/REPO}"; shift 2 ;;
        --path) SRC_PATH="${2:?--path needs a value}"; shift 2 ;;
        --ref)  SRC_REF="${2:?--ref needs a value}"; shift 2 ;;
        -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; exit 2 ;;
    esac
done

command -v gh >/dev/null 2>&1 || { echo "ERROR: gh CLI not found." >&2; exit 2; }

if [[ -z "$SRC_REPO" ]]; then
    echo "ERROR: no source repo configured." >&2
    echo "Pass --repo OWNER/REPO or set PII_NAMES_REPO to a PRIVATE repo." >&2
    echo "Never store customer names in a public repo." >&2
    exit 2
fi

api="repos/${SRC_REPO}/contents/${SRC_PATH}"
[[ -n "$SRC_REF" ]] && api="${api}?ref=${SRC_REF}"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

if ! gh api "$api" -H "Accept: application/vnd.github.raw" > "$tmp" 2>/dev/null; then
    echo "ERROR: could not fetch ${SRC_PATH} from ${SRC_REPO}." >&2
    echo "Check the repo/path, that it is private, and that you have access." >&2
    exit 2
fi

# Refuse to overwrite the local list with an empty/whitespace-only fetch.
if ! grep -qE '[^[:space:]]' "$tmp"; then
    echo "ERROR: fetched list from ${SRC_REPO}/${SRC_PATH} is empty; not overwriting ${DEST}." >&2
    exit 2
fi

mv "$tmp" "$DEST"
trap - EXIT
names="$(grep -vcE '^[[:space:]]*(#|$)' "$DEST" || true)"
echo "Wrote ${DEST} from ${SRC_REPO}/${SRC_PATH} (${names} name(s)). Names not printed."
echo "Maintainers: run 'just pii-sync' to push these to the CI secret."
