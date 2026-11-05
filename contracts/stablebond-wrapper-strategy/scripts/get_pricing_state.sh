#!/usr/bin/env bash
#
# Read the live pricing result off a deployed RWA yield-sharing strategy.
#
# READ-ONLY (simulated, `--send no`): submits no transaction, spends nothing, needs no keeper.
# `balance()` never reverts on pricing — this is the read that
# tells you WHY a number looks the way it does (deposits/invest/invest_otc fail closed outside
# Live while RWA is held; harvest emits SpreadAlertEvent instead of HarvestEvent;
# withdraw/divest/divest_otc keep working at the anchor).
#
# Exit codes (a bot loop should treat anything but 0 as "do not assume Live"):
#   0  mode == Live
#   2  mode is a recognized non-Live mode (OracleDown/Frozen)
#   3  the response could not be parsed into a recognized mode at all — fails CLOSED, not
#      silently healthy, if the CLI's enum JSON shape ever differs from what this script expects
#
# Usage:
#   Interactive:  ./get_pricing_state.sh
#   Positional:   ./get_pricing_state.sh <network> <source_account>
#
# <source_account> is any existing (on-chain) identity, used only to simulate the read.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "GET PRICING STATE (read)"

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

STATE="$(stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --send no \
  -- get_pricing_state)"

echo "──────────────────────────────────────"
echo " strategy: $STRATEGY_ID ($STRATEGY_KEY)"
echo "──────────────────────────────────────"

# Known PricingMode variants (src/pricing.rs). Anything else — jq missing, .mode absent/null,
# an enum shape this script doesn't recognize — is a PARSE FAILURE, not a green light: it must
# fail closed (exit 3), never fall through to "no news is good news".
is_known_mode() {
  case "$1" in
    Live | OracleDown | Frozen) return 0 ;;
    *) return 1 ;;
  esac
}

MODE=""
if command -v jq >/dev/null 2>&1; then
  printf '%s' "$STATE" | jq -r '
    "  mode:          \(.mode)",
    "  rate:          \(.rate)",
    "  raw_rate:      \(.raw_rate)",
    "  live_oracles:  \(.live_oracles)"
  ' 2>/dev/null || true
  # .mode is either a bare string (fieldless enum, the expected shape) or, if the CLI's
  # "friendly" output ever tags it, an object keyed by the variant name.
  MODE="$(printf '%s' "$STATE" | jq -r '
    if (.mode? | type) == "string" then .mode
    elif (.mode? | type) == "object" then (.mode | keys_unsorted[0] // "")
    else "" end
  ' 2>/dev/null || true)"
else
  echo "$STATE"
fi
echo "──────────────────────────────────────"

if ! is_known_mode "$MODE"; then
  echo "error: get_pricing_state could not parse a recognized PricingMode out of the response." >&2
  echo "  expected one of: Live OracleDown Frozen" >&2
  echo "  parsed mode: '${MODE:-<empty>}'" >&2
  echo "  raw response: $STATE" >&2
  echo "  treating this as NOT Live — do not assume the strategy is healthy." >&2
  exit 3
fi

if [[ "$MODE" != "Live" ]]; then
  echo "⚠  mode != Live ($MODE) — deposits/invest/invest_otc fail closed while RWA is held."
  exit 2
fi
