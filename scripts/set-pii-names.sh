#!/usr/bin/env bash
# set-pii-names.sh - Push the local PII name list to the PII_NAMES CI secret.
#
# The authoritative list lives in the gitignored .pii-names.local (one name per
# line; '#' comments and blank lines are ignored). CI reads the same names from
# the PII_NAMES secret. GitHub secrets are write-only, so you cannot append to
# one - re-run this after editing .pii-names.local to add or remove a name.
#
# Usage:
#   scripts/set-pii-names.sh            # org secret (all repos) - default
#   scripts/set-pii-names.sh --repo     # repo secret for this repo only
#   scripts/set-pii-names.sh --file P   # read the list from a different file
#
# The names are uploaded but never printed by this script.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPO_ROOT

# Edit these two constants if the org or repo ever changes.
readonly ORG="Strike48-public"
readonly REPO="Strike48-public/strikehub"

SCOPE="org"
LIST_FILE="${REPO_ROOT}/.pii-names.local"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo) SCOPE="repo"; shift ;;
        --org)  SCOPE="org"; shift ;;
        --file) LIST_FILE="${2:?--file needs a path}"; shift 2 ;;
        -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; exit 2 ;;
    esac
done

command -v gh >/dev/null 2>&1 || { echo "ERROR: gh CLI not found." >&2; exit 2; }

if [[ ! -f "$LIST_FILE" ]]; then
    echo "ERROR: name list not found: $LIST_FILE" >&2
    echo "Seed it from .pii-names.local.example, then re-run." >&2
    exit 2
fi

# Strip comment and blank lines; refuse to upload an empty list (which would
# make the scanner fail loud in CI on every run).
cleaned="$(grep -vE '^[[:space:]]*(#|$)' "$LIST_FILE" || true)"
count="$(printf '%s' "$cleaned" | grep -cE '[^[:space:]]' || true)"
if [[ "${count:-0}" -eq 0 ]]; then
    echo "ERROR: $LIST_FILE has no names (only comments/blanks)." >&2
    echo "Refusing to set an empty PII_NAMES secret." >&2
    exit 2
fi

echo "Uploading ${count} name(s) to the ${SCOPE} secret PII_NAMES..."
if [[ "$SCOPE" == "org" ]]; then
    printf '%s\n' "$cleaned" | gh secret set PII_NAMES --org "$ORG" --visibility all
    echo "Set org secret PII_NAMES on ${ORG} (visible to all repos)."
else
    printf '%s\n' "$cleaned" | gh secret set PII_NAMES --repo "$REPO"
    echo "Set repo secret PII_NAMES on ${REPO}."
fi
echo "Done. Re-run after editing ${LIST_FILE}; names are never printed."
