#!/usr/bin/env bash
#
# Update the pricing knobs on a deployed RWA yield-sharing strategy — both at once
# (the contract sets them in a single atomic write).
#
#   max_spread_bps  how far the rate may sit from the anchor `L`, either side, and so the most
#                    NAV can move per hour however wrong an oracle is. Range: 10..500
#                    (constants.rs MIN_SPREAD_BPS / MAX_SPREAD_BPS). THIS CALL ONLY LOWERS
#                    max_spread_bps — the contract returns NotAuthorized on a raise; a raise goes
#                    through ./propose_params.sh and waits 72h in public.
#   slippage_bps     execution tolerance applied to swap min_out / max_in around the oracle rate.
#                    Range: 0..500 (constants.rs MAX_SLIPPAGE_BPS). THIS CALL ONLY LOWERS
#                    slippage_bps too — same NotAuthorized-on-raise rule.
#
# ⚠  max_spread_bps and slippage_bps are safety limits, not performance knobs. Loosening either
#    to make a reverting call go through removes the protection the revert exists to provide —
#    and here you can't: only ./propose_params.sh can raise them, after a public 72h wait.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./set_pricing_params.sh
#   Positional:   ./set_pricing_params.sh <network> <source_account> \
#                        <max_spread_bps> <slippage_bps>
#
# The signer MUST be the strategy ADMIN (set_pricing_params calls admin.require_auth()).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "SET PRICING PARAMS"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

# Both are overwritten together, so show the live values: passing only the one you mean to
# change would silently reset the other.
CURRENT="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_params 2>/dev/null || true)"
if [[ -n "$CURRENT" ]] && command -v jq >/dev/null 2>&1; then
  echo "ℹ current: max_spread_bps=$(printf '%s' "$CURRENT" | jq -r '.max_spread_bps')" \
       "slippage_bps=$(printf '%s' "$CURRENT" | jq -r '.slippage_bps')"
  echo "  (BOTH are overwritten by this call — re-pass the one you don't want changed)"
  echo
fi

MAX_SPREAD_BPS=$(resolve_required "max_spread_bps (10..500)" "${3-__UNSET__}")
SLIPPAGE_BPS=$(resolve_required "slippage_bps (0..500)" "${4-__UNSET__}")

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

# Mirror the contract's bounds so a bad value fails here rather than on-chain.
[[ "$MAX_SPREAD_BPS" =~ ^[0-9]+$ ]] || die "max_spread_bps must be a non-negative integer, got '$MAX_SPREAD_BPS'"
[[ "$SLIPPAGE_BPS" =~ ^[0-9]+$ ]] || die "slippage_bps must be a non-negative integer, got '$SLIPPAGE_BPS'"
(( MAX_SPREAD_BPS >= 10 && MAX_SPREAD_BPS <= 500 )) \
  || die "max_spread_bps out of range [10, 500], got '$MAX_SPREAD_BPS'"
(( SLIPPAGE_BPS <= 500 )) \
  || die "slippage_bps out of range [0, 500], got '$SLIPPAGE_BPS'"

echo
echo "──────────────────────────────────────"
echo " network:            $NETWORK"
echo " signer:             $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:           $STRATEGY_ID ($STRATEGY_KEY)"
echo " max_spread_bps:     $MAX_SPREAD_BPS"
echo " slippage_bps:       $SLIPPAGE_BPS"
echo " inclusion fee:      $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- set_pricing_params \
  --max_spread_bps "$MAX_SPREAD_BPS" \
  --slippage_bps "$SLIPPAGE_BPS"

echo
echo "✅ pricing params updated"
