#!/usr/bin/env bash
#
# Start a two-step admin transfer on a deployed RWA yield-sharing strategy.
#
# Sets pending_admin; nothing changes until the NEW admin calls ./accept_admin.sh themselves.
# Losing the admin on this non-upgradable contract loses every knob (caps, pricing, keeper,
# oracle/OTC proposals), so there is no one-shot rotation — a typo here costs nothing, since
# the old admin stays admin until the new address proves it can sign.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./transfer_admin.sh <network> <admin_identity> <new_admin>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "TRANSFER ADMIN"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
NEW_ADMIN=$(resolve_required "New admin address" "${3-__UNSET__}")
[[ "$NEW_ADMIN" =~ ^[GC][A-Z2-7]{55}$ ]] \
  || die "new_admin must look like a Stellar address (G... account or C... contract), got '$NEW_ADMIN'"
INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " new_admin:  $NEW_ADMIN  (pending until they accept_admin.sh)"
echo "──────────────────────────────────────"
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- transfer_admin --new_admin "$NEW_ADMIN"
echo "✅ transfer started — pending_admin = $NEW_ADMIN until they run accept_admin.sh"
