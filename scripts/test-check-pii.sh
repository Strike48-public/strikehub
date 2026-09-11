#!/usr/bin/env bash
# test-check-pii.sh - Regression tests for scripts/check-pii.sh.
#
# Runs the scanner through its exit-code contract, list-parsing, word-boundary
# matching, redaction, and - most importantly - the metacharacter regression:
# a banned name containing an unbalanced '(' or '[' must still be caught, and
# must never silently pass (that was the original defect this suite guards).
#
# No external dependencies (no bats). Exit 0 if all pass, 1 otherwise.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCANNER="${SCRIPT_DIR}/check-pii.sh"

pass=0
fail=0

# An empty regular file used as PII_NAMES_FILE forces the "no list" path
# regardless of whether a developer has a real .pii-names.local present
# (PII_NAMES_FILE is consulted before .pii-names.local).
EMPTY_LIST="$(mktemp)"
trap 'rm -f "$EMPTY_LIST"' EXIT

# expect_exit <want> <desc> <names> <input> [PII_REDACT]
# Runs the scanner on <input> via stdin with PII_NAMES=<names>; asserts exit code.
expect_exit() {
    local want="$1" desc="$2" names="$3" input="$4" redact="${5:-}"
    local rc=0
    printf '%s\n' "$input" | env -u PII_NAMES_FILE PII_NAMES="$names" PII_REDACT="$redact" \
        "$SCANNER" >/dev/null 2>&1 || rc=$?
    if [[ "$rc" == "$want" ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        printf 'FAIL: %s (want exit %s, got %s)\n' "$desc" "$want" "$rc" >&2
    fi
}

# --- exit-code contract ---
# No list configured -> fail loud (exit 2), even if a .pii-names.local exists.
no_list_rc=0
printf 'anything\n' | env -u PII_NAMES PII_NAMES_FILE="$EMPTY_LIST" "$SCANNER" >/dev/null 2>&1 || no_list_rc=$?
if [[ "$no_list_rc" == 2 ]]; then pass=$((pass + 1)); else
    fail=$((fail + 1)); printf 'FAIL: no list -> exit 2 (got %s)\n' "$no_list_rc" >&2
fi

expect_exit 1 "detect a name"                 "AcmeCorp"      "uses AcmeCorp here"
expect_exit 0 "clean input"                   "AcmeCorp"      "nothing to see"
expect_exit 1 "case-insensitive"              "AcmeCorp"      "acmecorp lower"
expect_exit 1 "comma-separated list"          "Foo,Bar,Baz"   "has Bar in it"
expect_exit 0 "word-boundary (no substring)"  "Acme"          "Acmestical is fine"
expect_exit 1 "multi-word name"               "Acme Corp"     "the Acme Corp tenant"

# --- metacharacter regression (the core guard) ---
# Each banned name below contains a regex metacharacter. With the old raw-regex
# scanner these silently passed (exit 0); fixed-string matching must catch them.
expect_exit 1 "unbalanced '(' in name"        'Acme(Corp'     "leak Acme(Corp data"
expect_exit 1 "unbalanced '[' in name"        'Acme[Inc'      "leak Acme[Inc data"
expect_exit 1 "unbalanced ')' in name"        'Foo)Bar'       "leak Foo)Bar data"
expect_exit 1 "one bad name must not disable others" $'RealCustomer\nFoo)Bar' "leak RealCustomer secret"
# A literal '.' must match a literal dot, not an arbitrary character.
expect_exit 1 "literal dot matches dot"       'acme.io'       "tenant acme.io here"
expect_exit 0 "literal dot is not wildcard"   'acme.io'       "unrelated acmeXio text"

# --- redaction: the matched customer name must never appear in output ---
redact_out="$(printf 'title uses AcmeCorp\n' \
    | env -u PII_NAMES_FILE PII_NAMES="AcmeCorp" PII_REDACT=1 "$SCANNER" 2>&1 || true)"
if grep -q "AcmeCorp" <<< "$redact_out"; then
    fail=$((fail + 1)); printf 'FAIL: redaction leaked the name into output\n' >&2
else
    pass=$((pass + 1))
fi

printf '\n%s passed, %s failed\n' "$pass" "$fail"
[[ "$fail" -eq 0 ]]
