#!/usr/bin/env bash
#
# Read the immutable Config off a deployed RWA yield-sharing strategy.
#
# READ-ONLY (simulated, `--send no`): submits no transaction, spends nothing, needs no keeper.
# Returns the tokens, their cached decimals, and the fixed venue — set once at construction and
# never changed afterwards (the venue has no setter; a pool migration is a new
# deployment).
#
# Exit codes:
#   0  parsed successfully (venue kind recognized as Aqua or Sushi)
#   3  the response could not be parsed into a recognized Venue kind — fails CLOSED: the raw
#      JSON is printed to stderr instead of silently showing a partial or wrong summary
#
# Usage:
#   Interactive:  ./get_config.sh
#   Positional:   ./get_config.sh <network> <source_account>
#
# <source_account> is any existing (on-chain) identity, used only to simulate the read.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "GET CONFIG (read)"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Read identity (any existing account; only simulates)" "${2-__UNSET__}")

ensure_network
stellar keys ls 2>/dev/null | grep -qw "$SOURCE_ACCOUNT" \
  || die "identity '$SOURCE_ACCOUNT' not found (stellar keys ls)"
SIGNER_PUBKEY="$(stellar keys public-key "$SOURCE_ACCOUNT")"
echo "✓ read identity '$SOURCE_ACCOUNT' → $SIGNER_PUBKEY"
load_strategy_id
echo

CONFIG="$(stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --send no \
  -- get_config)"

echo "──────────────────────────────────────"
echo " strategy: $STRATEGY_ID ($STRATEGY_KEY)"
echo "──────────────────────────────────────"

# Known Venue variants (src/storage.rs). Anything else — jq missing, .venue absent/null, an
# enum shape this script doesn't recognize — is a PARSE FAILURE: fail closed (exit 3) rather
# than silently printing a summary that looks complete but isn't, mirroring
# get_pricing_state.sh's mode handling.
is_known_venue_kind() {
  case "$1" in
    Aqua | Sushi) return 0 ;;
    *) return 1 ;;
  esac
}

VENUE_KIND=""
if command -v jq >/dev/null 2>&1; then
  printf '%s' "$CONFIG" | jq -r '
    "  asset:          \(.asset) (\(.asset_decimals) dp)",
    "  rwa_asset:      \(.rwa_asset) (\(.rwa_decimals) dp)",
    "  venue:          \(.venue)"
  ' 2>/dev/null || true
  # .venue is expected as an object keyed by the variant name (Aqua/Sushi), the CLI's usual
  # external-tagged shape for a tuple enum variant; fall back gracefully for a bare string.
  VENUE_KIND="$(printf '%s' "$CONFIG" | jq -r '
    if (.venue? | type) == "object" then (.venue | keys_unsorted[0] // "")
    elif (.venue? | type) == "string" then .venue
    else "" end
  ' 2>/dev/null || true)"
else
  echo "$CONFIG"
fi
echo "──────────────────────────────────────"

if ! is_known_venue_kind "$VENUE_KIND"; then
  echo "error: get_config could not parse a recognized Venue kind out of the response." >&2
  echo "  expected one of: Aqua Sushi" >&2
  echo "  parsed venue kind: '${VENUE_KIND:-<empty>}'" >&2
  echo "  raw response: $CONFIG" >&2
  exit 3
fi
