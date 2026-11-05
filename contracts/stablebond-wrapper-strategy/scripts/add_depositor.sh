#!/usr/bin/env bash
#
# Allowlist a depositor (the DeFindex vault) on a deployed RWA yield-sharing strategy.
#
# The strategy ships fail-closed: deposits are rejected until the ADMIN allowlists
# the vault. This is that step.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./add_depositor.sh
#   Positional:   ./add_depositor.sh <network> <source_account> <depositor>
#
# The signer MUST be the strategy ADMIN (add_depositor calls admin.require_auth()).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "ADD DEPOSITOR"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the strategy admin)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

DEPOSITOR=$(resolve_required "Depositor to allowlist (vault contract id / address)" "${3-__UNSET__}")

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:    $NETWORK"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " depositor:  $DEPOSITOR"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Allowlist this depositor now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- add_depositor \
  --depositor "$DEPOSITOR"

echo
echo "✅ Allowlisted depositor $DEPOSITOR"
