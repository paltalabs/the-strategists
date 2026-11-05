import type { Contracts } from './config';
import type { StrategyState } from './data';
import { formatBps, formatRate, formatUnits } from './format';
import { AddressLink, Card, Row } from './ui';

const MODE_HINT: Record<string, string> = {
  Live: 'At least one oracle is live; the rate is the oracle price clamped around the anchor.',
  OracleDown: 'No live oracle; the rate holds at the anchor.',
  Frozen: 'Price frozen; the rate holds at the anchor.',
};

function formatTs(ts: bigint): string {
  return new Date(Number(ts) * 1000).toISOString().replace('T', ' ').slice(0, 16) + ' UTC';
}

export function StrategyCard(props: { contracts: Contracts; strategy: StrategyState | null; error: string | null }) {
  const s = props.strategy;

  return (
    <Card title="Strategy · RWA yield" error={props.error} aside={<AddressLink address={props.contracts.strategy} />}>
      {!s ? (
        <p className="muted">{props.error ? 'No data yet; retrying on the next refresh.' : 'Loading…'}</p>
      ) : (
        <>
          <div className="stats">
            <div className="stat">
              <span className="stat-label">Pricing mode</span>
              <span className={`badge badge-${s.mode.toLowerCase()}`} title={MODE_HINT[s.mode]}>
                {s.mode}
              </span>
            </div>
            <div className="stat">
              <span className="stat-label">NAV per {s.rwaSymbol}</span>
              <span className="stat-value">
                {formatRate(s.rate, s.assetDecimals)} <small>{s.usdcSymbol}</small>
              </span>
            </div>
          </div>
          <Row label="Oracle rate (before clamp)">{formatRate(s.rawRate, s.assetDecimals)}</Row>
          <Row label="Anchor" hint="Last valid price; moves at most the max spread per hour">
            {s.anchor === null ? 'not set yet' : formatRate(s.anchor, s.assetDecimals)}
            {s.anchorTs !== null && <span className="muted small"> · {formatTs(s.anchorTs)}</span>}
          </Row>
          {s.mode === 'Frozen' && (
            <Row label="Frozen until">
              {/* An admin freeze is u64::MAX: it lasts until lifted. */}
              {s.frozenUntil >= 18_446_744_073_709_551_615n ? 'until lifted by admin' : formatTs(s.frozenUntil)}
            </Row>
          )}
          <Row label="Live oracles">
            {s.liveOracles} / {s.oracleCount}
          </Row>
          <h3>Holdings</h3>
          <Row label={`Idle ${s.usdcSymbol}`}>{formatUnits(s.idleUsdc, s.assetDecimals)} {s.usdcSymbol}</Row>
          <Row label={`${s.rwaSymbol} held`}>{formatUnits(s.rwaHeld, s.rwaDecimals, 4)}</Row>
          <Row label="Invested value">{formatUnits(s.rwaValue, s.assetDecimals)} {s.usdcSymbol}</Row>
          <Row label="Strategy NAV">{formatUnits(s.idleUsdc + s.rwaValue, s.assetDecimals)} {s.usdcSymbol}</Row>
          <h3>Limits</h3>
          <Row label="Max per swap">{formatUnits(s.maxSwap, s.assetDecimals)} {s.usdcSymbol}</Row>
          <Row label="Max invested">{formatUnits(s.maxInvested, s.assetDecimals)} {s.usdcSymbol}</Row>
          <Row label="Max spread / slippage">
            {formatBps(BigInt(s.maxSpreadBps))} / {formatBps(BigInt(s.slippageBps))}
          </Row>
          <Row label="Venue">{s.venue}</Row>
          <Row label="Deposits">{s.paused ? 'paused' : 'open'}</Row>
          <Row label="Auto-invest">{s.autoInvest ? 'on' : 'off (keeper invests)'}</Row>
        </>
      )}
    </Card>
  );
}
