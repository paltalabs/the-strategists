#!/usr/bin/env bash
#
# Harvest a deployed RWA yield-sharing strategy.
#
# THIS IS A TELEMETRY NO-OP, NOT A COMPOUNDING CALL. There is nothing to claim: this strategy
# earns by holding a value-accruing RWA whose price rises, not by accruing a claimable reward
# token. Harvest exists to stamp an on-chain datapoint:
#   - priced OK  -> HarvestEvent { amount: 0, price_per_share } in Live mode, which off-chain
#                   APY indexers diff across time
#   - priced BAD -> SpreadAlertEvent (plus the PricingModeEvent the guarded pricing emits). It
#                   does NOT revert — an alert is the product here.
# It moves no funds and mints no shares. The trait signature carries an optional `data` payload
# (there is no reward to name, so it is always `null`).
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./harvest.sh
#   Positional:   ./harvest.sh <network> <source_account>
#
# The signer MUST be the strategy keeper: harvest is dual-gated — keeper.require_auth() AND
# from == keeper — so the keeper is passed as `from` automatically.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "HARVEST (telemetry)"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the strategy keeper)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:    $NETWORK"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " from:       $SIGNER_PUBKEY  (must equal the on-chain keeper)"
echo " data:       null  (no rewards to claim — telemetry only)"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"
echo "→ moves no funds; emits HarvestEvent (price_per_share) in Live mode, SpreadAlertEvent otherwise."

read -rp "Harvest now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- harvest \
  --from "$SIGNER_PUBKEY" \
  --data null

echo
echo "✅ harvested (telemetry emitted)"
