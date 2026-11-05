#!/usr/bin/env bash
#
# Narrow the OTC price band (Params.otc_max_deviation_bps) on a deployed RWA yield-sharing strategy.
#
#   otc_max_deviation_bps  the furthest, in basis points either side of the gated rate, that the
#                          keeper's per-fill price on invest_otc / divest_otc may sit. The
#                          room for passing a counterparty's quote through, not a fee on assets
#                          under management. Cap MAX_OTC_DEVIATION_BPS = 500 (5%), shipped 0 (fills at exactly
#                          the rate).
#
# ⚠  THIS CALL ONLY NARROWS otc_max_deviation_bps — the contract returns NotAuthorized on a raise.
#    Widening it goes through ./propose_params.sh and waits 72h in public.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./set_otc_max_deviation_bps.sh
#   Positional:   ./set_otc_max_deviation_bps.sh <network> <source_account> <bps>
#
# The signer MUST be the strategy ADMIN (set_otc_max_deviation_bps calls admin.require_auth()).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "SET OTC MAX DEVIATION BPS"

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
  echo "ℹ current: otc_max_deviation_bps=$(printf '%s' "$CURRENT" | jq -r '.otc_max_deviation_bps')"
  echo "  (this call only NARROWS — widening it needs ./propose_params.sh)"
  echo
fi

BPS=$(resolve_required "otc_max_deviation_bps (0..500, must be <= current)" "${3-__UNSET__}")
[[ "$BPS" =~ ^[0-9]+$ ]] || die "otc_max_deviation_bps must be a non-negative integer, got '$BPS'"
(( BPS <= 500 )) || die "otc_max_deviation_bps out of range [0, 500] (MAX_OTC_DEVIATION_BPS), got '$BPS'"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:          $NETWORK"
echo " signer:            $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:          $STRATEGY_ID ($STRATEGY_KEY)"
echo " otc_max_deviation_bps:   $BPS"
echo " inclusion fee:     $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- set_otc_max_deviation_bps \
  --bps "$BPS"

echo
echo "✅ otc_max_deviation_bps = $BPS"
