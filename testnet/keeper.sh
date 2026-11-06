#!/usr/bin/env bash
# Keeps the testnet demo live for reviewers. Each tick:
#   1. pushes the TBOND NAV, grown at NAV_APY_BPS a year (the oracle expires an hour after
#      the last push, which would put the strategy in OracleDown)
#   2. arbitrages the pool back toward NAV with the LP account when it drifts past ARB_BAND_BPS
#   3. moves idle vault USDC into the strategy (kept at par; the RWA is bought OTC),
#      keeping a buffer idle in the vault for withdrawals
#   4. every FEES_EVERY_S, runs the revenue share: report, lock_fees, distribute_fees
# Every step is best effort: a failure is logged and the next tick tries again.
# Usage: ./keeper.sh once | ./keeper.sh loop [seconds, default 1800]
source "$(dirname "${BASH_SOURCE[0]}")/_env.sh"

NAV_APY_BPS=500
ARB_BAND_BPS=50
ARB_STEP_USDC=10000
VAULT_BUFFER_USDC=5000
FEES_EVERY_S=21600

USDC="$(state_get usdc)"; RWA="$(state_get rwa)"; POOL="$(state_get pool)"
STRATEGY="$(state_get strategy)"; VAULT="$(state_get vault)"
LP="$(addr "$LP_ID")"; A="$(state_get admin)"

py() { python3 -I -c "$@"; }
raw() { tr -d '"\n'; }
stamp() { echo "[$(date -u '+%F %T')] $*"; }
units() { py 'import sys; print(int(sys.argv[1])*10**7)' "$1"; }

# NAV grown by NAV_APY_BPS a year, pro rata for the seconds since the last push.
grow_nav() {
  local nav last now
  nav="$(state_get nav)"; last="$(state_get nav_pushed_at)"; now="$(date +%s)"
  [[ -n "$last" ]] || last="$now"
  py 'import sys; n,a,dt=map(int,sys.argv[1:]); print(n + n*a*dt//(10000*31536000))' "$nav" "$NAV_APY_BPS" "$((now - last))"
}

push_nav() {
  local nav; nav="$(grow_nav)"
  "$HERE/push_nav.sh" "$nav" 2>/dev/null >/dev/null && state_set nav_pushed_at "$(date +%s)" && stamp "NAV pushed: $nav"
}

# Pool price in USDC per TBOND, as an integer with 14 decimals like the oracle NAV.
pool_price() {
  local sqrt; sqrt="$(view "$POOL" -- slot0 | jq -r .sqrt_price_x96)"
  py 'import sys; s=int(sys.argv[1]); t0=sys.argv[2]=="true"; p=s*s*10**14//2**192; print(p if t0 else 10**28//p)' "$sqrt" "$(state_get rwa_is_token0)"
}

# One swap of ARB_STEP_USDC worth toward NAV, repeated while outside the band (max 10).
arb() {
  local nav i price dev zfo amount lim hints t0
  nav="$(state_get nav)"; t0="$(state_get rwa_is_token0)"
  for i in $(seq 1 10); do
    price="$(pool_price)"
    dev="$(py 'import sys; p,n=map(int,sys.argv[1:]); print((p-n)*10000//n)' "$price" "$nav")"
    (( dev > -ARB_BAND_BPS && dev < ARB_BAND_BPS )) && return 0
    if (( dev < 0 )); then   # pool under NAV: buy TBOND with USDC
      [[ "$t0" == true ]] && zfo=false || zfo=true
      amount="$(units "$ARB_STEP_USDC")"
    else                     # pool over NAV: sell TBOND for USDC
      [[ "$t0" == true ]] && zfo=true || zfo=false
      amount="$(py 'import sys; print(int(sys.argv[1])*10**7*10**14//int(sys.argv[2]))' "$ARB_STEP_USDC" "$nav")"
    fi
    lim="$([[ "$zfo" == true ]] && echo 4295128740 || echo 1461446703485210103287273052203988822378723970341)"
    hints="$(view "$POOL" -- get_oracle_hints)"
    inv "$POOL" "$LP_ID" -- swap --sender "$LP" --recipient "$LP" --zero_for_one "$zfo" --amount_specified "$amount" \
      --sqrt_price_limit_x96 "$lim" --hints "$hints" >/dev/null 2>&1 || { stamp "arb swap failed"; return 1; }
    stamp "arb: pool was ${dev} bps from NAV, swapped $ARB_STEP_USDC TUSDC worth"
  done
}

allocate() {
  local idle step
  idle="$(view "$USDC" -- balance --id "$VAULT" | raw)"
  step="$(py 'import sys; i,b=map(int,sys.argv[1:]); print(max(0,i-b))' "$idle" "$(units "$VAULT_BUFFER_USDC")")"
  if (( step > 0 )); then
    inv "$VAULT" "$ADMIN_ID" -- rebalance --caller "$A" --instructions "[{\"Invest\":[\"$STRATEGY\",\"$step\"]}]" >/dev/null 2>&1 \
      && stamp "vault moved $step raw TUSDC into the strategy" || stamp "rebalance failed"
  fi
  # No keeper buys on the pool: they pay the pool's premium over NAV plus its fee, which
  # shows up as a mark-to-market loss. Idle strategy USDC is invested OTC at the oracle rate
  # instead (invest_otc), once a counterparty is allowlisted.
}

fees() {
  local last now; last="$(state_get fees_at)"; now="$(date +%s)"
  [[ -n "$last" ]] && (( now - last < FEES_EVERY_S )) && return 0
  inv "$VAULT" "$ADMIN_ID" -- report >/dev/null 2>&1 \
    && inv "$VAULT" "$ADMIN_ID" -- lock_fees >/dev/null 2>&1 \
    && inv "$VAULT" "$ADMIN_ID" -- distribute_fees --caller "$A" >/dev/null 2>&1 \
    && state_set fees_at "$now" && stamp "revenue share distributed" || stamp "fees failed"
}

tick() {
  push_nav || stamp "NAV push failed"
  arb || true
  allocate
  fees
}

case "${1:-}" in
  once) tick ;;
  loop) while true; do tick; sleep "${2:-1800}"; done ;;
  *) sed -n 2,11p "$0"; exit 1 ;;
esac
