/// 1 with 12 decimal places. Scales `rate` and the `price_per_share` harvest metric.
pub const SCALAR_12: i128 = 1_000_000_000_000;

/// Virtual-offset constant for ERC-4626-style inflation-attack resistance. Added to the
/// shares side of every share<->asset conversion.
pub const VIRTUAL_SHARES: i128 = 1_000_000;

/// Basis-points denominator. 100% = 10_000 bps.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Bounds on `max_spread_bps`: 0.1% .. 5%. The spread is how far the rate may sit from the
/// anchor and so how far any oracle, however wrong, can move NAV per `ANCHOR_WINDOW_SECONDS`
///.
pub const MIN_SPREAD_BPS: u32 = 10;
pub const MAX_SPREAD_BPS: u32 = 500;

/// Wait between proposing and executing the three changes that add trust: a new oracle set,
/// a new OTC counterparty, or a raise of any of the four price knobs (`propose_params`).
/// 72 h, long enough for the vault manager to pause the strategy and unwind.
pub const TIMELOCK_SECONDS: u64 = 259_200;

/// The `Live`-mode anchor (`LastValidPrice`) is rewritten at most once per window, and each
/// rewrite moves it at most `max_spread_bps`. Caps how fast a wrong oracle can walk NAV
///.
pub const ANCHOR_WINDOW_SECONDS: u64 = 3_600;

/// How long a keeper's `freeze_price` holds the rate at the anchor before it lapses on its own.
/// A frozen rate pays exits at the anchor even if NAV truly fell, so a compromised keeper must
/// not be able to hold it there indefinitely: the keeper gets one freeze, re-armed only by an
/// admin unfreeze or `set_keeper`, and 24 h gives the admin multisig time to confirm the freeze
/// (its own is indefinite) or rotate the keeper.
pub const KEEPER_FREEZE_SECONDS: u64 = 86_400;

/// Upper bound on any oracle source's staleness window. 24 h: the RedStone
/// push feeds write on a 12 h heartbeat or a 0.1% deviation that a NAV asset (~0.01%/day)
/// rarely reaches, so every shipped preset sets the RedStone TTL to this constant, 2x the
/// heartbeat.
pub const MAX_TTL_SECONDS: u64 = 86_400;

/// Hard cap on `slippage_bps` (5%): an uncapped value would let a careless or compromised
/// keeper authorize near-total-loss swaps with one call.
pub const MAX_SLIPPAGE_BPS: u32 = 500;

/// Hard cap on `Params.otc_max_deviation_bps` (a band around the gated rate): the furthest
/// an OTC fill price may sit from the rate, either side. 500 because primary mint and redeem
/// prices for these RWAs are unknown ahead of launch and the contract cannot be upgraded; the
/// stored band ships at 0 and any raise still waits `TIMELOCK_SECONDS`. Same ceiling as
/// `MAX_SPREAD_BPS`, so the `MAX_RATE` overflow bound below covers it unchanged.
pub const MAX_OTC_DEVIATION_BPS: u32 = 500;

/// Upper bound on any cached `decimals()` value (token or oracle). An oversized value would
/// make every later `pow10` exceed the CPU budget on a non-upgradable contract.
pub const MAX_DECIMALS: u32 = 18;

/// Upper bound on the oracle set size. Every source is one `try_` call on every pricing.
pub const MAX_ORACLES: u32 = 3;

/// The largest rate pricing accepts. An oracle whose normalized rate exceeds it is
/// "not live", exactly like a stale or panicking one. The ceiling keeps
/// `math::spread_bps` (`|a - b| * 10_000`), `math::apply_bps` (`x * (10_000 +/- s)`, with
/// `s <= MAX_SPREAD_BPS`), and the OTC band `rate * (1 ± otc_max_deviation_bps)` — the band capped
/// far below it — inside i128 for every rate pricing can see, so `price_rwa` cannot return
/// `ArithmeticError` for an external reason and `balance()` stays total.
pub const MAX_RATE: i128 = i128::MAX / 20_000;

/// Strategy name carried as the `#[topic]` on every emitted event.
pub const STRATEGY_NAME: &str = "StableBondWrapperStrategy";

/// The most one `reanchor_to_oracle` may lower the anchor (5%), and that call is allowed once
/// per `ANCHOR_WINDOW_SECONDS`. Bounds what a compromised admin multisig with a wrong-low
/// oracle can take at once; a real drop past it books over several windows.
pub const MAX_REANCHOR_STEP_BPS: u32 = 500;
