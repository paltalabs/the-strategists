#!/usr/bin/env bash
#
# Propose new values for the four price knobs — max_spread_bps, slippage_bps, max_swap and
# otc_max_deviation_bps — on a deployed RWA yield-sharing strategy. This is the only way any of the
# four goes UP: a raise here waits TIMELOCK_SECONDS (72h) and is executable only via
# ./execute_params.sh, after ParamsProposedEvent { max_spread_bps, slippage_bps, max_swap,
# otc_max_deviation_bps, executable_at } gives public notice carrying every proposed value. Lowering
# any of the four is immediate instead — use ./set_caps.sh / ./set_pricing_params.sh /
# ./set_otc_max_deviation_bps.sh.
#
# All four values are proposed together, replacing any earlier pending proposal — this is not a
# per-field update. There is no get_pending_params.sh; the read is one invoke:
#
#   stellar contract invoke --id <strategy_id> --source-account <any_identity> \
#     --network <network> --send no -- get_pending_params
#
# which is exactly what ./execute_params.sh and ./cancel_params.sh run inline before they submit.
#
# Usage:
#   DEPLOYMENT=<ustry|usdy|dejtrsy> ./propose_params.sh <network> <admin_identity> \
#     <max_spread_bps> <slippage_bps> <max_swap> <otc_max_deviation_bps>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "PROPOSE PARAMS"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo

CURRENT="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_params 2>/dev/null || true)"
if [[ -n "$CURRENT" ]] && command -v jq >/dev/null 2>&1; then
  echo "ℹ current: max_spread_bps=$(printf '%s' "$CURRENT" | jq -r '.max_spread_bps')" \
       "slippage_bps=$(printf '%s' "$CURRENT" | jq -r '.slippage_bps')" \
       "max_swap=$(printf '%s' "$CURRENT" | jq -r '.max_swap')" \
       "otc_max_deviation_bps=$(printf '%s' "$CURRENT" | jq -r '.otc_max_deviation_bps')"
  echo
fi

MAX_SPREAD_BPS=$(resolve_required "max_spread_bps (10..500)" "${3-__UNSET__}")
SLIPPAGE_BPS=$(resolve_required "slippage_bps (0..500)" "${4-__UNSET__}")
MAX_SWAP=$(resolve_required "max_swap (stroops, positive)" "${5-__UNSET__}")
OTC_BAND_BPS=$(resolve_required "otc_max_deviation_bps (0..500)" "${6-__UNSET__}")

# Mirror the contract's bounds so a bad value fails here rather than on-chain. Any value may be
# proposed, lower ones included — the wait is what makes a raise public, not the direction.
require_positive_i128 "max_swap" "$MAX_SWAP"
[[ "$MAX_SPREAD_BPS" =~ ^[0-9]+$ ]] || die "max_spread_bps must be a non-negative integer, got '$MAX_SPREAD_BPS'"
[[ "$SLIPPAGE_BPS" =~ ^[0-9]+$ ]] || die "slippage_bps must be a non-negative integer, got '$SLIPPAGE_BPS'"
[[ "$OTC_BAND_BPS" =~ ^[0-9]+$ ]] || die "otc_max_deviation_bps must be a non-negative integer, got '$OTC_BAND_BPS'"
(( MAX_SPREAD_BPS >= 10 && MAX_SPREAD_BPS <= 500 )) \
  || die "max_spread_bps out of range [10, 500], got '$MAX_SPREAD_BPS'"
(( SLIPPAGE_BPS <= 500 )) \
  || die "slippage_bps out of range [0, 500], got '$SLIPPAGE_BPS'"
(( OTC_BAND_BPS <= 500 )) \
  || die "otc_max_deviation_bps out of range [0, 500] (MAX_OTC_DEVIATION_BPS), got '$OTC_BAND_BPS'"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " strategy:        $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:          $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " max_spread_bps:  $MAX_SPREAD_BPS"
echo " slippage_bps:    $SLIPPAGE_BPS"
echo " max_swap:        $MAX_SWAP stroops"
echo " otc_max_deviation_bps: $OTC_BAND_BPS"
echo "──────────────────────────────────────"
echo "⚠  This starts a public 72h wait; these values take effect only via execute_params.sh."
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

AT="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- propose_params \
  --max_spread_bps "$MAX_SPREAD_BPS" \
  --slippage_bps "$SLIPPAGE_BPS" \
  --max_swap "$MAX_SWAP" \
  --otc_max_deviation_bps "$OTC_BAND_BPS" | tr -d '"')"

DATE="$(date -u -r "$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -d "@$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "$AT")"
echo "✅ proposed — executable_at = $AT ($DATE)"
