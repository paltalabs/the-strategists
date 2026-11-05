#!/usr/bin/env bash
#
# Update max_swap and max_invested on a deployed RWA yield-sharing strategy — both at once
# (the contract sets them in a single atomic write).
#
#   max_swap      USDC per swap: invest tranche, divest tranche, withdraw shortfall. Bounds our
#                  own price impact on the venue. Re-derive from live pool depth, don't
#                  raise it just to unstick a reverting call. THIS CALL ONLY LOWERS max_swap — the
#                  contract returns NotAuthorized on a raise; a raise goes through
#                  ./propose_params.sh and waits 72h in public.
#   max_invested   Cap on the USDC value of held RWA. Bounds the share of a pool we own. Moves
#                   freely either way, no timelock — it caps exposure, not price.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./set_caps.sh
#   Positional:   ./set_caps.sh <network> <source_account> <max_swap> <max_invested>
#
# The signer MUST be the strategy ADMIN (set_caps calls admin.require_auth()).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "SET CAPS"

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
  echo "ℹ current: max_swap=$(printf '%s' "$CURRENT" | jq -r '.max_swap')" \
       "max_invested=$(printf '%s' "$CURRENT" | jq -r '.max_invested')"
  echo "  (BOTH are overwritten by this call — re-pass the one you don't want changed)"
  echo
fi

MAX_SWAP=$(resolve_required "max_swap (stroops, positive)" "${3-__UNSET__}")
MAX_INVESTED=$(resolve_required "max_invested (stroops, positive)" "${4-__UNSET__}")
require_positive_i128 "max_swap" "$MAX_SWAP"
require_positive_i128 "max_invested" "$MAX_INVESTED"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:       $NETWORK"
echo " signer:        $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:      $STRATEGY_ID ($STRATEGY_KEY)"
echo " max_swap:      $MAX_SWAP stroops"
echo " max_invested:  $MAX_INVESTED stroops"
echo " inclusion fee: $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- set_caps \
  --max_swap "$MAX_SWAP" \
  --max_invested "$MAX_INVESTED"

echo
echo "✅ max_swap = $MAX_SWAP, max_invested = $MAX_INVESTED"
