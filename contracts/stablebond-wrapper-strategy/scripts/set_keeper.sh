#!/usr/bin/env bash
#
# Replace the keeper on a deployed RWA yield-sharing strategy.
#
# The keeper is the bot role: invest / divest / OTC fills / harvest / pause.
# rotation is admin-only and IMMEDIATE — a single transaction, no new-keeper counter-signature.
# That is deliberate: the bot key must be replaceable in one transaction when
# compromised, and the keeper cannot move funds to an address it chose, so there is no typo
# risk worth a two-party handshake here.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./set_keeper.sh
#   Positional:   ./set_keeper.sh <network> <source_account> <new_keeper>
#
# The signer MUST be the strategy ADMIN (set_keeper calls admin.require_auth()).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "SET KEEPER"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

# Show the on-chain current keeper so the operator can confirm before rotating.
CURRENT_KEEPER="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_params 2>/dev/null | (jq -r '.keeper' 2>/dev/null || true) || true)"
[[ -n "$CURRENT_KEEPER" && "$CURRENT_KEEPER" != "null" ]] && echo "ℹ current on-chain keeper: $CURRENT_KEEPER"

NEW_KEEPER=$(resolve_required "New keeper address" "${3-__UNSET__}")
[[ "$NEW_KEEPER" =~ ^[GC][A-Z2-7]{55}$ ]] \
  || die "new_keeper must look like a Stellar address (G... account or C... contract), got '$NEW_KEEPER'"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:         $NETWORK"
echo " signer:          $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:        $STRATEGY_ID ($STRATEGY_KEY)"
echo " current keeper:  ${CURRENT_KEEPER:-<unknown>}"
echo " NEW keeper:      $NEW_KEEPER"
echo " inclusion fee:   $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"
echo "⚠  Takes effect IMMEDIATELY in this one transaction. If \$NEW_KEEPER is an address you"
echo "   do not control, the keeper role — invest/divest/OTC fills/harvest/pause — moves there."

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- set_keeper \
  --new_keeper "$NEW_KEEPER"

echo
echo "✅ Keeper rotated to $NEW_KEEPER"
