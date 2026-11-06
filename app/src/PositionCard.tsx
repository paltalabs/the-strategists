import { useState } from 'react';
import { addTrustline, invoke } from './chain';
import type { Contracts } from './config';
import { classicAsset, type Position, type VaultState } from './data';
import { errorMessage, formatUnits, parseUnits } from './format';
import { Card, Row, TxNotice, type TxStatus } from './ui';
import { signerFor } from './wallet';

const SHARE_DECIMALS = 7;
/** Withdraw tolerance: accept up to 1% less than the current value of the shares. */
const WITHDRAW_TOLERANCE_BPS = 100n;

interface Props {
  contracts: Contracts;
  address: string | null;
  vault: VaultState | null;
  position: Position | null;
  error: string | null;
  onTxDone: () => void;
}

function valueOfShares(shares: bigint, vault: VaultState): bigint {
  return vault.totalSupply > 0n ? (shares * vault.totalManaged) / vault.totalSupply : 0n;
}

export function PositionCard(props: Props) {
  const { contracts, address, vault, position } = props;
  const [depositInput, setDepositInput] = useState('');
  const [withdrawInput, setWithdrawInput] = useState('');
  const [withdrawMode, setWithdrawMode] = useState<'shares' | 'percent'>('percent');
  const [tx, setTx] = useState<TxStatus | null>(null);
  const busy = tx?.kind === 'pending';

  if (!address) {
    return (
      <Card title="My position">
        <p className="muted">Connect a wallet to see your balance and deposit or withdraw.</p>
      </Card>
    );
  }
  if (!vault || !position) {
    return (
      <Card title="My position" error={props.error}>
        <p className="muted">{props.error ? 'No data yet; retrying on the next refresh.' : 'Loading…'}</p>
      </Card>
    );
  }

  const sym = vault.usdc.symbol;
  const d = vault.usdc.decimals;
  const myValue = valueOfShares(position.shares, vault);
  const asset = classicAsset(vault.usdc.name);

  async function run(label: string, send: () => Promise<string>): Promise<void> {
    setTx({ kind: 'pending', label });
    try {
      const hash = await send();
      setTx({ kind: 'ok', label: `${label} confirmed`, hash });
      props.onTxDone();
    } catch (err) {
      setTx({ kind: 'error', label: `${label} failed`, message: errorMessage(err) });
    }
  }

  function deposit(): void {
    const amount = parseUnits(depositInput, d);
    if (amount === null || amount <= 0n) {
      setTx({ kind: 'error', label: 'Deposit', message: 'Enter a positive amount.' });
      return;
    }
    if (amount > position!.usdcBalance) {
      setTx({ kind: 'error', label: 'Deposit', message: `You only have ${formatUnits(position!.usdcBalance, d)} ${sym}.` });
      return;
    }
    // invest=false: funds stay idle in the vault; the strategy's keeper invests them.
    void run('Deposit', () =>
      invoke(
        contracts.vault,
        'deposit',
        { amounts_desired: [amount], amounts_min: [amount], from: address, invest: false },
        address!,
        signerFor(address!),
      ),
    );
  }

  function sharesToWithdraw(): bigint | null {
    if (withdrawMode === 'shares') return parseUnits(withdrawInput, SHARE_DECIMALS);
    const pct = Number(withdrawInput);
    if (!Number.isFinite(pct) || pct <= 0 || pct > 100) return null;
    // Exact 100% burns every share, avoiding rounding dust.
    return pct === 100 ? position!.shares : (position!.shares * BigInt(Math.round(pct * 100))) / 10_000n;
  }

  function withdraw(): void {
    const shares = sharesToWithdraw();
    if (shares === null || shares <= 0n) {
      setTx({ kind: 'error', label: 'Withdraw', message: 'Enter a valid amount of shares or a percentage (0-100].' });
      return;
    }
    if (shares > position!.shares) {
      setTx({ kind: 'error', label: 'Withdraw', message: 'That is more shares than you hold.' });
      return;
    }
    const expected = valueOfShares(shares, vault!);
    const minOut = (expected * (10_000n - WITHDRAW_TOLERANCE_BPS)) / 10_000n;
    void run('Withdraw', () =>
      invoke(
        contracts.vault,
        'withdraw',
        { withdraw_shares: shares, min_amounts_out: [minOut], from: address },
        address!,
        signerFor(address!),
      ),
    );
  }

  function trust(): void {
    if (!asset) return;
    void run('Add trustline', () => addTrustline(address!, asset.code, asset.issuer, signerFor(address!)));
  }

  const preview = sharesToWithdraw();

  return (
    <Card title="My position" error={props.error}>
      {!position.accountExists && (
        <p className="warn">
          This account is not funded on testnet. Fund it with XLM first:{' '}
          <a href={`https://friendbot.stellar.org/?addr=${address}`} target="_blank" rel="noreferrer">
            Friendbot
          </a>
          , then refresh.
        </p>
      )}
      {position.trustline === false && asset && (
        <div className="warn">
          Your account has no trustline to {asset.code}, so it cannot hold test {sym}.{' '}
          <button className="btn btn-small" onClick={trust} disabled={busy}>
            Add trustline
          </button>
        </div>
      )}
      <div className="stats">
        <div className="stat">
          <span className="stat-label">Wallet {sym}</span>
          <span className="stat-value">{formatUnits(position.usdcBalance, d)}</span>
        </div>
        <div className="stat">
          <span className="stat-label">My value</span>
          <span className="stat-value">
            {formatUnits(myValue, d)} <small>{sym}</small>
          </span>
        </div>
      </div>
      <Row label="My vault shares">{formatUnits(position.shares, SHARE_DECIMALS, 4)}</Row>

      <div className="forms">
        <div className="form">
          <h3>Deposit</h3>
          <div className="input-row">
            <input
              inputMode="decimal"
              placeholder={`Amount in ${sym}`}
              value={depositInput}
              onChange={(e) => setDepositInput(e.target.value)}
            />
            <button className="btn btn-ghost btn-small" onClick={() => setDepositInput(formatUnits(position.usdcBalance, d, d).replace(/,/g, ''))}>
              Max
            </button>
          </div>
          <button className="btn" onClick={deposit} disabled={busy || !position.accountExists}>
            Deposit
          </button>
        </div>

        <div className="form">
          <h3>Withdraw</h3>
          <div className="toggle">
            <button className={withdrawMode === 'percent' ? 'on' : ''} onClick={() => setWithdrawMode('percent')}>
              %
            </button>
            <button className={withdrawMode === 'shares' ? 'on' : ''} onClick={() => setWithdrawMode('shares')}>
              Shares
            </button>
          </div>
          <div className="input-row">
            <input
              inputMode="decimal"
              placeholder={withdrawMode === 'percent' ? 'Percent of my shares' : 'Shares'}
              value={withdrawInput}
              onChange={(e) => setWithdrawInput(e.target.value)}
            />
            <button className="btn btn-ghost btn-small" onClick={() => setWithdrawInput(withdrawMode === 'percent' ? '100' : formatUnits(position.shares, SHARE_DECIMALS, SHARE_DECIMALS).replace(/,/g, ''))}>
              Max
            </button>
          </div>
          {preview !== null && preview > 0n && preview <= position.shares && (
            <p className="muted small">
              ≈ {formatUnits(valueOfShares(preview, vault), d)} {sym} (min {formatUnits((valueOfShares(preview, vault) * (10_000n - WITHDRAW_TOLERANCE_BPS)) / 10_000n, d)})
            </p>
          )}
          <button className="btn" onClick={withdraw} disabled={busy || position.shares === 0n}>
            Withdraw
          </button>
        </div>
      </div>
      <TxNotice status={tx} />
      <p className="muted small">
        Test {sym} comes from the faucet ("Get test tokens"). A classic-asset token needs a trustline before your
        account can receive it.
      </p>
    </Card>
  );
}
