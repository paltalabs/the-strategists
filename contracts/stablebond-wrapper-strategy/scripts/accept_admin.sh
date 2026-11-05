#!/usr/bin/env bash
#
# Complete a two-step admin transfer on a deployed RWA yield-sharing strategy.
#
# The signer MUST be the pending admin — accept_admin calls pending_admin.require_auth(). Prints
# the pending admin from get_params first so you can confirm the signer matches before signing.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./accept_admin.sh <network> <pending_admin_identity>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "ACCEPT ADMIN"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the pending admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo

PENDING="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_params 2>/dev/null | (jq -r '.pending_admin // empty' 2>/dev/null || true) || true)"
[[ -n "$PENDING" ]] || die "no admin transfer is pending on this strategy (get_params.pending_admin is empty)"
echo "ℹ pending_admin on-chain: $PENDING"
[[ "$PENDING" == "$SIGNER_PUBKEY" ]] \
  || echo "⚠  signer ($SIGNER_PUBKEY) does not match pending_admin ($PENDING) — this will fail on-chain."

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:  $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:    $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo "──────────────────────────────────────"
read -rp "Accept admin now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- accept_admin
echo "✅ admin accepted — $SIGNER_PUBKEY is now the admin"
