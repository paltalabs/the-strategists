#!/usr/bin/env bash
#
# Keeper: buy one tranche of RWA with idle USDC on a deployed RWA yield-sharing strategy.
# Refused when paused, outside Live pricing, above max_swap, above idle or above the
# max_invested room. min_out is the oracle-implied quantity minus slippage, so a
# pool under NAV fills above it (an accretive buy) and a pool above NAV reverts.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./invest.sh <network> <keeper_identity> <usdc_stroops>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "INVEST"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the keeper)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
USDC_AMOUNT=$(resolve_required "usdc_amount (stroops, <= max_swap)" "${3-__UNSET__}")
require_positive_i128 "usdc_amount" "$USDC_AMOUNT"
INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " usdc_amount $USDC_AMOUNT stroops"
echo "──────────────────────────────────────"
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- invest --usdc_amount "$USDC_AMOUNT"
echo "✅ invested"
