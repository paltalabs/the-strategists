import { useState } from 'react';
import type { Contracts } from './config';
import { simulateReport, type Report, type VaultState } from './data';
import { errorMessage, formatUnits } from './format';
import { AddressLink, Card, Row } from './ui';

/** Shares the vault mints to itself on the first deposit (vault `MINIMUM_LIQUIDITY`). */
const MINIMUM_LIQUIDITY = 1000n;

/** Percent with two decimals from a numerator over 10_000 * 10_000 (bps of bps). */
function pctOfBpsProduct(product: number): string {
  return `${(product / 1_000_000).toFixed(2)}%`;
}

function RevenueSplit(props: { vaultFeeBps: number; protocolRateBps: number }) {
  const vf = props.vaultFeeBps;
  const pr = props.protocolRateBps;
  // The protocol rate is a share OF the vault fee, not an extra charge on top of it.
  const parts = [
    { name: 'Depositors keep', pct: pctOfBpsProduct((10_000 - vf) * 10_000), cls: 'seg-user', w: (10_000 - vf) * 10_000 },
    { name: 'Partner', pct: pctOfBpsProduct(vf * (10_000 - pr)), cls: 'seg-partner', w: vf * (10_000 - pr) },
    { name: 'Infrastructure', pct: pctOfBpsProduct(vf * pr), cls: 'seg-infra', w: vf * pr },
  ];
  return (
    <div className="split">
      <div className="split-bar">
        {parts.map((p) =>
          p.w > 0 ? <div key={p.name} className={`seg ${p.cls}`} style={{ flexGrow: p.w }} title={`${p.name} ${p.pct}`} /> : null,
        )}
      </div>
      <ul className="split-legend">
        {parts.map((p) => (
          <li key={p.name}>
            <span className={`dot ${p.cls}`} />
            {p.name} <strong>{p.pct}</strong> <span className="muted">of gains</span>
          </li>
        ))}
      </ul>
      <p className="muted small">
        Vault fee {(vf / 100).toFixed(2)}% of gains; of that fee, {(pr / 100).toFixed(2)}% goes to infrastructure and the
        rest to the partner.
      </p>
    </div>
  );
}

function ReportPreview(props: { contracts: Contracts; decimals: number }) {
  const [reports, setReports] = useState<Report[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function run(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      setReports(await simulateReport(props.contracts));
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="report">
      <button className="btn btn-ghost" onClick={run} disabled={busy}>
        {busy ? 'Simulating…' : 'Preview fees (simulate report)'}
      </button>
      {error && <p className="error">{error}</p>}
      {reports?.map((r, i) => (
        <div key={i} className="report-rows">
          <Row label="Unrealized gains / losses" hint="Gain since the last fee lock, not yet charged">
            {formatUnits(r.gains_or_losses, props.decimals, 4)}
          </Row>
          <Row label="Locked fees" hint="Fees already locked, awaiting distribution">
            {formatUnits(r.locked_fee, props.decimals, 4)}
          </Row>
        </div>
      ))}
    </div>
  );
}

export function VaultCard(props: { contracts: Contracts; vault: VaultState | null; error: string | null }) {
  const v = props.vault;
  const sym = v?.usdc.symbol ?? 'USDC';
  const d = v?.usdc.decimals ?? 7;
  // Vault shares have 7 decimals (hardcoded in the vault); price is USDC per share.
  const sharePrice = v && v.totalSupply > 0n ? (v.totalManaged * 10_000_000n) / v.totalSupply : null;
  // Only the vault's own locked first-deposit shares remain: every depositor has exited and
  // whatever is left (dust, unclaimed fees) sits on those 0.0001 shares, so the per-share
  // price is not meaningful.
  const onlyLockedShares = v !== null && v.totalSupply <= MINIMUM_LIQUIDITY;

  return (
    <Card title="Vault" error={props.error} aside={<AddressLink address={props.contracts.vault} />}>
      {!v ? (
        <p className="muted">{props.error ? 'No data yet; retrying on the next refresh.' : 'Loading…'}</p>
      ) : (
        <>
          <div className="stats">
            <div className="stat">
              <span className="stat-label">TVL</span>
              <span className="stat-value">
                {formatUnits(v.totalManaged, d)} <small>{sym}</small>
              </span>
            </div>
            <div className="stat">
              <span className="stat-label">Share price</span>
              <span className="stat-value">
                {sharePrice === null || onlyLockedShares ? '—' : formatUnits(sharePrice, d, 6)} <small>{sym}</small>
              </span>
            </div>
          </div>
          {onlyLockedShares && (
            <p className="muted small">
              No depositors right now: only the vault&apos;s locked minimum shares exist, so share price is not shown.
            </p>
          )}
          <Row label="Idle in vault">{formatUnits(v.idle, d)} {sym}</Row>
          <Row label="Invested in strategy" hint="Net of locked fees">
            {formatUnits(v.invested, d)} {sym}
          </Row>
          <Row label="Total shares">{formatUnits(v.totalSupply, 7, 7)}</Row>
          <h3>Revenue split</h3>
          <RevenueSplit vaultFeeBps={v.vaultFeeBps} protocolRateBps={v.protocolRateBps} />
          <Row label="Fee receiver (on-chain)">
            <AddressLink address={v.feeReceiver} />
          </Row>
          <Row label="Partner">
            <AddressLink address={props.contracts.partner} />
          </Row>
          <Row label="Infrastructure receiver">
            <AddressLink address={props.contracts.defindex_receiver} />
          </Row>
          <ReportPreview contracts={props.contracts} decimals={d} />
        </>
      )}
    </Card>
  );
}
