#!/usr/bin/env bash
#
# Read the mutable Params off a deployed RWA yield-sharing strategy.
#
# READ-ONLY (simulated, `--send no`): submits no transaction, spends nothing, needs no keeper.
# Returns the whole Params struct — admin, keeper, pending_admin, the oracle set, caps, pricing
# knobs, auto_invest and the pause flag. Written wholesale by the setters, one atomic write.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./get_params.sh
#   Positional:   ./get_params.sh <network> <source_account>
#
# <source_account> is any existing (on-chain) identity, used only to simulate the read.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "GET PARAMS (read)"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Read identity (any existing account; only simulates)" "${2-__UNSET__}")

ensure_network
# A read needs the source only to exist on-chain (for simulation) — not to be funded.
stellar keys ls 2>/dev/null | grep -qw "$SOURCE_ACCOUNT" \
  || die "identity '$SOURCE_ACCOUNT' not found (stellar keys ls)"
SIGNER_PUBKEY="$(stellar keys public-key "$SOURCE_ACCOUNT")"
echo "✓ read identity '$SOURCE_ACCOUNT' → $SIGNER_PUBKEY"
load_strategy_id
echo

PARAMS="$(stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --send no \
  -- get_params)"

echo "──────────────────────────────────────"
echo " strategy: $STRATEGY_ID ($STRATEGY_KEY)"
echo "──────────────────────────────────────"

# Pretty-print when jq is around; fall back to the raw JSON otherwise. Caps are i128 stroops;
# the 7-decimal hint matches USDC, the underlying on every planned config.
if command -v jq >/dev/null 2>&1; then
  printf '%s' "$PARAMS" | jq -r '
    "  admin:              \(.admin)",
    "  pending_admin:      \(.pending_admin // "<none>")",
    "  keeper:             \(.keeper)",
    "  paused:             \(.paused)",
    "  auto_invest:        \(.auto_invest)",
    "  max_swap:           \(.max_swap) stroops (~\((.max_swap|tonumber)/10000000) if 7dp)",
    "  max_invested:       \(.max_invested) stroops (~\((.max_invested|tonumber)/10000000) if 7dp)",
    "  max_spread_bps:     \(.max_spread_bps) (\((.max_spread_bps|tonumber)/100)%)",
    "  slippage_bps:       \(.slippage_bps) (\((.slippage_bps|tonumber)/100)%)",
    "  frozen_until:       \(.frozen_until) (0 = not frozen; 18446744073709551615 = admin freeze)",
    "  oracles:            \(.oracles | length) source(s)"
  '
  echo "──────────────────────────────────────"
  echo " summary: $(printf '%s' "$PARAMS" | jq -c '{admin, keeper, paused, auto_invest, oracle_count: (.oracles | length)}')"
else
  echo "$PARAMS"
fi
echo "──────────────────────────────────────"
