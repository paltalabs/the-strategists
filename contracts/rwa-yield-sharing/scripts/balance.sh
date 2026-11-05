#!/usr/bin/env bash
#
# Read a depositor's underlying-denominated balance on a deployed RWA yield-sharing strategy.
#
# This is READ-ONLY (simulated, `--send no`): it submits no transaction, spends nothing, and
# needs no keeper. `balance(from)` returns `from`'s share of the strategy NAV in the
# underlying's stroops (floored — conservative toward the strategy). Right after a deposit it
# is ~= the amount deposited (deposits mint shares at NAV and swap nothing, so only share
# rounding is lost); it grows over time as the held RWA appreciates against USDC.
#
# NOTE: `balance` NEVER REVERTS on pricing — it stays total and side-effect free in
# every mode, pricing at the lowest live oracle clamped around the anchor, or at the
# anchor itself when no oracle is live or the price is frozen. A degraded or stale number here is
# a signal, not a script failure: run ./get_pricing_state.sh to see WHY the number looks the way
# it does (mode, live_oracles).
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./balance.sh
#   Positional:   ./balance.sh <network> <source_account> [depositor_address]
#
# <source_account> is any existing (on-chain) identity used only to simulate the read; its
# public key is the default depositor to query. Pass [depositor_address] to query someone else.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "BALANCE (read)"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Read identity (any existing account; only simulates)" "${2-__UNSET__}")

ensure_network
# A read needs the source only to exist on-chain (for simulation) — not to be funded — so we
# resolve its pubkey directly instead of the stricter ensure_signer balance check.
stellar keys ls 2>/dev/null | grep -qw "$SOURCE_ACCOUNT" \
  || die "identity '$SOURCE_ACCOUNT' not found (stellar keys ls)"
SIGNER_PUBKEY="$(stellar keys public-key "$SOURCE_ACCOUNT")"
echo "✓ read identity '$SOURCE_ACCOUNT' → $SIGNER_PUBKEY"
load_strategy_id
echo

FROM=$(resolve_with_default "Depositor address to query" "$SIGNER_PUBKEY" "${3-__UNSET__}")

BAL="$(stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --send no \
  -- balance --from "$FROM" | tr -d '"')"

# Human-readable hint assuming the underlying has 7 decimals (USDC on this deployment).
HUMAN="$(awk -v s="$BAL" 'BEGIN { printf "%.7f", s / 10000000 }' 2>/dev/null || echo "?")"

echo
echo "──────────────────────────────────────"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " depositor:  $FROM"
echo " balance:    $BAL stroops  (~$HUMAN if 7 decimals / USDC)"
echo "──────────────────────────────────────"
