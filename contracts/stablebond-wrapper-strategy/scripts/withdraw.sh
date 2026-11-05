#!/usr/bin/env bash
#
# Withdraw underlying from a deployed RWA yield-sharing strategy.
#
# The signer IS the position holder (`from`): withdraw calls `from.require_auth()` and
# burns `from`'s shares. Withdraw is NOT gated by the allowlist and NEVER pausable, so a
# de-listed vault can still exit. The underlying is sent to `to` (defaults to the signer).
#
# There is no per-deposit cap to worry about here — the constraint is on the SHORTFALL: idle
# USDC is served first, and only the amount beyond idle is sold for RWA. That shortfall sale
# must be at most max_swap or the whole call reverts with InsufficientBalance; the
# keeper's idle buffer, not a larger max_swap, is what keeps large exits serviceable. Proceeds
# in any pricing mode, at the mode's bounded rate.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./withdraw.sh
#   Positional:   ./withdraw.sh <network> <source_account> <amount> <to>
#
# Pass "-" for <to> to default it to the signer's own address.
# <amount> is raw stroops (positive i128) in the underlying's decimals.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "WITHDRAW"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (the position holder / vault)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

AMOUNT=$(resolve_required "Amount (positive i128 stroops)" "${3-__UNSET__}")
require_positive_i128 "amount" "$AMOUNT"

TO=$(resolve_with_default "Recipient address" "$SIGNER_PUBKEY" "${4-__UNSET__}")

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:   $NETWORK"
echo " signer:    $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:  $STRATEGY_ID ($STRATEGY_KEY)"
echo " from:      $SIGNER_PUBKEY"
echo " to:        $TO"
echo " amount:    $AMOUNT"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Withdraw now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- withdraw \
  --amount "$AMOUNT" \
  --from "$SIGNER_PUBKEY" \
  --to "$TO"

echo
echo "✅ Withdrew $AMOUNT to $TO"
