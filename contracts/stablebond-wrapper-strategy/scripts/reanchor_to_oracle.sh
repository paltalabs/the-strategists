#!/usr/bin/env bash
#
# Admin: book a CONFIRMED NAV drop fast by lowering the anchor (LastValidPrice) toward the
# oracle rate. Without it, Live pricing books a drop one max_spread_bps step
# per hour (~10 h per 1% at the shipped 10 bps), and every exit in between is paid at the
# higher NAV. Each call moves the anchor at most 5% (MAX_REANCHOR_STEP_BPS) and one call is
# allowed per hour, so a larger drop needs a call per hour: run this again each hour with the
# same figure until get_pricing_state shows the anchor at the oracle (5 calls for 20%).
#
# Use it only after the drop is confirmed off-chain (issuer NAV page, redemption quote): pass
# that figure as the expected rate. The contract refuses unless
#   - the price is not frozen (unfreeze first with ./freeze_price.sh ... false; that also
#     re-arms the keeper's one freeze, src/lib.rs:716, so IF THE KEEPER IS SUSPECT run
#     ./set_keeper.sh BEFORE the unfreeze: set_keeper is immediate, src/lib.rs:643, and a
#     compromised keeper could otherwise refreeze for 24 h between the unfreeze and this call),
#   - no reanchor in the last hour,
#   - every configured oracle is live and fresh,
#   - every oracle is within max_spread_bps of the expected rate,
#   - the lowest oracle is strictly under the current anchor.
# It can only LOWER NAV; recovery comes back through the normal clamp, one step per hour.
# Policy and ops steps: docs/strategies/rwa-yield-sharing/architecture.md, "Impairment policy".
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy>.
#
# Usage:
#   Interactive:  ./reanchor_to_oracle.sh
#   Positional:   ./reanchor_to_oracle.sh <network> <admin_identity> <expected_rate>
#
# expected_rate is in the unit of get_pricing_state().rate: USDC stroops per whole RWA x 1e12
#, e.g. 0.80 USDC per RWA = 8000000000000000000.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "REANCHOR TO ORACLE"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Admin identity" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

STATE="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_pricing_state 2>/dev/null || true)"
if [[ -n "$STATE" ]]; then
  echo "→ current pricing state (rate = the clamped NAV, raw_rate = the lowest live oracle):"
  echo "$STATE"
fi

EXPECTED=$(resolve_required "Expected rate you confirmed off-chain (rate units)" "${3-__UNSET__}")
require_positive_i128 "expected rate" "$EXPECTED"

if [[ -n "$STATE" ]] && command -v python3 >/dev/null 2>&1; then
  RAW="$(_otc_json_field "$STATE" raw_rate)"
  RATE="$(_otc_json_field "$STATE" rate)"
  if [[ -n "$RAW" && -n "$RATE" && "$RATE" != "0" ]]; then
    python3 - "$RATE" "$RAW" "$EXPECTED" <<'PY'
import sys
rate, raw, exp = (int(x) for x in sys.argv[1:4])
print(f"ℹ the lowest oracle is {(raw - exp) * 10_000 / exp:+.2f} bps from your figure; "
      f"NAV moves {(raw - rate) * 10_000 / rate:+.2f} bps")
PY
  fi
fi

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:        $NETWORK"
echo " signer:         $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:       $STRATEGY_ID ($STRATEGY_KEY)"
echo " expected rate:  $EXPECTED"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"
echo "→ NAV drops NOW, to the oracle rate or 5% under the anchor, whichever is higher. Every vault holder's balance falls with it."

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- reanchor_to_oracle \
  --expected_rate "$EXPECTED"

echo
echo "✅ reanchor_to_oracle sent — ./get_pricing_state.sh shows the new rate"
