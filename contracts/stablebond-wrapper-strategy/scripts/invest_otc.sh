#!/usr/bin/env bash
#
# Keeper: OTC-buy RWA from an allowlisted counterparty at the price agreed with it.
# The contract refuses any price further than otc_max_deviation_bps from the gated rate (ships
# 0: the price must equal the rate). Needs at least one live oracle: the bounded rate is then
# never above the last agreed price L, so a lying oracle cannot widen what the keeper may pay.
# Never bounded by max_swap (no venue impact) but capped by the invested room under max_invested.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./invest_otc.sh <network> <keeper_identity> <counterparty> <usdc_amount> <price|rate>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "INVEST OTC"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the keeper)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
CP=$(resolve_required "Counterparty address (must be allowlisted)" "${3-__UNSET__}")
[[ "$CP" =~ ^[GC][A-Z2-7]{55}$ ]] \
  || die "counterparty must look like a Stellar address (G... account or C... contract), got '$CP'"
AMT=$(resolve_required "usdc_amount (stroops, <= idle and invested room)" "${4-__UNSET__}")
require_positive_i128 "usdc_amount" "$AMT"
echo
resolve_otc_price "${5-__UNSET__}"

STATE="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_pricing_state)"
if command -v jq >/dev/null 2>&1; then
  LIVE_ORACLES="$(printf '%s' "$STATE" | jq -r '.live_oracles // 0' 2>/dev/null || echo 0)"
  [[ "$LIVE_ORACLES" =~ ^[0-9]+$ ]] || LIVE_ORACLES=0
  (( LIVE_ORACLES > 0 )) \
    || die "live_oracles is 0 — invest_otc needs at least one live oracle."
fi

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo
echo "──────────────────────────────────────"
echo " strategy:     $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:       $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " counterparty: $CP"
echo " usdc_amount:  $AMT stroops"
echo " price:        $OTC_PRICE"
echo "──────────────────────────────────────"
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- invest_otc --counterparty "$CP" --usdc_amount "$AMT" --price "$OTC_PRICE"
echo "✅ invested OTC"
