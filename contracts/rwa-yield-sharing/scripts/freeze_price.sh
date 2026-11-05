#!/usr/bin/env bash
#
# Freeze / unfreeze the price of a deployed RWA yield-sharing strategy at its anchor
# (the last Live rate, `LastValidPrice`).
#
# This is the stop for a WRONG ORACLE. Replacing an oracle waits 72h (propose_oracles.sh); a
# freeze is immediate and holds NAV at the anchor while that wait runs. Frozen:
#   - balance(), withdraw, divest and divest_otc price at the anchor;
#   - deposits while RWA is held, invest and invest_otc are refused.
# Freezing can only hold the last agreed price, never set one. After a freeze ends the rate
# rejoins the oracle at most max_spread_bps per hour.
#
# A KEEPER freeze lasts 24h (KEEPER_FREEZE_SECONDS) and lapses on its own, so a compromised bot
# cannot hold exits at a stale price. It is ONE-SHOT: a second keeper freeze is refused
# (NotAuthorized) until the admin unfreezes with `false` or calls set_keeper, which re-arm it.
# The ADMIN confirms a keeper freeze by freezing too (indefinite) or lifts it with `false`.
# Only the admin can unfreeze.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./freeze_price.sh
#   Positional:   ./freeze_price.sh <network> <source_account> <true|false>
#
# Freezing: the signer may be the ADMIN or the KEEPER. Unfreezing: ADMIN only.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "FREEZE PRICE"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (admin or keeper to freeze, admin to unfreeze)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

FROZEN=$(resolve_required "Frozen (true = hold the price at the anchor, false = follow the oracle again)" "${3-__UNSET__}")
[[ "$FROZEN" == "true" || "$FROZEN" == "false" ]] \
  || die "expected 'true' or 'false', got '$FROZEN'"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:    $NETWORK"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " caller:     $SIGNER_PUBKEY"
echo " frozen:     $FROZEN"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"
if [[ "$FROZEN" == "true" ]]; then
  echo "→ NAV held at the anchor. Withdrawals and divest_otc stay open at that price;"
  echo "  deposits (while RWA is held), invest and invest_otc are REJECTED."
else
  echo "→ the rate follows the oracle again, at most max_spread_bps per hour from the anchor."
fi

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- freeze_price \
  --caller "$SIGNER_PUBKEY" \
  --frozen "$FROZEN"

echo
echo "✅ freeze_price($FROZEN) sent — ./get_params.sh shows frozen_until"
