#!/usr/bin/env bash
#
# Keeper: OTC-sell RWA to an allowlisted counterparty for exactly usdc_amount, at the price
# agreed with it, refused when further than otc_max_deviation_bps from the gated
# rate, the oracle NAV (ships 0: the price must equal the rate). Any pricing mode with a rate
# (OracleDown and Frozen price at the anchor): this is how idle is refilled without selling
# into the pool. Refuses rather than
# short-paying when the RWA leg it implies exceeds what's held. Never bounded by max_swap.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./divest_otc.sh <network> <keeper_identity> <counterparty> <usdc_amount> <price|rate>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "DIVEST OTC"
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
AMT=$(resolve_required "usdc_amount (stroops)" "${4-__UNSET__}")
require_positive_i128 "usdc_amount" "$AMT"
echo
resolve_otc_price "${5-__UNSET__}"

# Preview the RWA leg it implies (informational — the contract computes the exact
# amount on-chain; this is qty_for(usdc, price, rwa_decimals) = usdc * 10^rwa_decimals * 1e12 / price).
if command -v jq >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
  RWA_DECIMALS="$(stellar contract invoke \
    --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
    -- get_config 2>/dev/null | jq -r '.rwa_decimals // empty' 2>/dev/null || true)"
  if [[ -n "$RWA_DECIMALS" ]]; then
    RWA_LEG="$(python3 -c "print((int('$AMT') * (10 ** int('$RWA_DECIMALS')) * 1_000_000_000_000) // int('$OTC_PRICE'))" 2>/dev/null || true)"
    echo "ℹ at price $OTC_PRICE: ~$AMT stroops implies ~$RWA_LEG raw RWA units sold, at $RWA_DECIMALS dp"
  fi
else
  echo "ℹ (install jq + python3 to preview the RWA leg this implies)"
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
  -- divest_otc --counterparty "$CP" --usdc_amount "$AMT" --price "$OTC_PRICE"
echo "✅ divested OTC"
