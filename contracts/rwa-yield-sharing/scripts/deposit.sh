#!/usr/bin/env bash
#
# Deposit underlying into a deployed RWA yield-sharing strategy.
#
# The signer IS the depositor (`from`): the strategy pulls tokens via
# `token::transfer(from, strategy, amount)` and calls `from.require_auth()`, so the
# signer needs a sufficient underlying balance. The signer must ALSO be allowlisted
# (see add_depositor.sh) or the deposit reverts with NotAuthorized.
#
# There is NO per-deposit cap: the whole amount lands as idle
# USDC and mints at par against pre-deposit NAV; only afterwards, if auto_invest is on and
# pricing is Live, does a best-effort tranche of at most max_swap get bought. Fails closed
# outside Live while RWA is already held (a stale anchor would misprice the shares).
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./deposit.sh
#   Positional:   ./deposit.sh <network> <source_account> <amount>
#
# <amount> is raw stroops (positive i128) in the underlying's decimals.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "DEPOSIT"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (the depositor / vault)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

AMOUNT=$(resolve_required "Amount (positive i128 stroops)" "${3-__UNSET__}")
require_positive_i128 "amount" "$AMOUNT"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:   $NETWORK"
echo " signer:    $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:  $STRATEGY_ID ($STRATEGY_KEY)"
echo " from:      $SIGNER_PUBKEY"
echo " amount:    $AMOUNT"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Deposit now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- deposit \
  --amount "$AMOUNT" \
  --from "$SIGNER_PUBKEY"

echo
echo "✅ Deposited $AMOUNT (from $SIGNER_PUBKEY)"
