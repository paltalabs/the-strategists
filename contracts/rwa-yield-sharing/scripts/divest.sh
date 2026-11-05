#!/usr/bin/env bash
#
# Keeper: sell RWA to raise exactly usdc_target USDC into idle on a deployed
# RWA yield-sharing strategy. One tranche per call, refused above max_swap. Not
# allowlist-gated, not pausable — the RWA-side counterpart to invest.sh. Run this after a large
# withdrawal to refill the keeper's idle buffer so the next withdrawal's shortfall
# stays under max_swap.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./divest.sh <network> <keeper_identity> <usdc_target_stroops>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "DIVEST"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the keeper)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
USDC_TARGET=$(resolve_required "usdc_target (stroops, idle target)" "${3-__UNSET__}")
require_positive_i128 "usdc_target" "$USDC_TARGET"
INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:    $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:      $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " usdc_target: $USDC_TARGET stroops"
echo "──────────────────────────────────────"
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- divest --usdc_target "$USDC_TARGET"
echo "✅ divested"
