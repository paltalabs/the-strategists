// Admin tab: one card per role, with a button per privileged method. A button is enabled
// only when the connected wallet holds the role, since the contract would reject anyone else.

import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { invoke } from './chain';
import type { Contracts } from './config';
import { loadRoles, type Roles, type StrategyState, type VaultState } from './data';
import { errorMessage, formatUnits, parseUnits } from './format';
import { AddressLink, Card, Row, TxNotice, type TxStatus } from './ui';
import { signerFor } from './wallet';

interface Props {
  contracts: Contracts;
  address: string | null;
  vault: VaultState | null;
  strategy: StrategyState | null;
  onTxDone: () => void;
}

/** Sends one transaction signed by the connected wallet and tracks its status. */
function useTx(address: string | null, onDone: () => void) {
  const [status, setStatus] = useState<TxStatus | null>(null);
  const run = useCallback(
    async (label: string, contractId: string, method: string, args: Record<string, unknown>): Promise<void> => {
      if (!address) {
        setStatus({ kind: 'error', label, message: 'Connect a wallet first.' });
        return;
      }
      setStatus({ kind: 'pending', label });
      try {
        const hash = await invoke(contractId, method, args, address, signerFor(address));
        setStatus({ kind: 'ok', label: `${label} confirmed`, hash });
        onDone();
      } catch (err) {
        setStatus({ kind: 'error', label: `${label} failed`, message: errorMessage(err) });
      }
    },
    [address, onDone],
  );
  return { status, setStatus, run, busy: status?.kind === 'pending' };
}

function RoleRow(props: { label: string; holder: string | null; address: string | null }) {
  const you = props.holder !== null && props.holder === props.address;
  return (
    <Row label={props.label}>
      {props.holder ? <AddressLink address={props.holder} /> : <span className="muted">unknown</span>}
      {you && <span className="badge badge-live you">you</span>}
    </Row>
  );
}

function Action(props: { title: string; hint?: string; children: ReactNode }) {
  return (
    <div className="action">
      <h3 title={props.hint}>{props.title}</h3>
      {props.hint && <p className="muted small action-hint">{props.hint}</p>}
      {props.children}
    </div>
  );
}

/** Amount input plus a submit button, parsed with `decimals`. */
function AmountAction(props: {
  title: string;
  hint?: string;
  unit: string;
  decimals: number;
  button: string;
  disabled: boolean;
  initial?: string;
  onSubmit: (amount: bigint) => void;
  onInvalid: () => void;
}) {
  const [value, setValue] = useState(props.initial ?? '');
  useEffect(() => {
    if (props.initial !== undefined && value === '') setValue(props.initial);
    // Only prefill once the initial value arrives; never overwrite what the user typed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.initial]);
  function submit(): void {
    const amount = parseUnits(value, props.decimals);
    if (amount === null || amount <= 0n) props.onInvalid();
    else props.onSubmit(amount);
  }
  return (
    <Action title={props.title} hint={props.hint}>
      <div className="input-row">
        <input inputMode="decimal" placeholder={props.unit} value={value} onChange={(e) => setValue(e.target.value)} />
        <button className="btn btn-small" onClick={submit} disabled={props.disabled}>
          {props.button}
        </button>
      </div>
    </Action>
  );
}

function NotHolder(props: { show: boolean; role: string }) {
  if (!props.show) return null;
  return <p className="warn">Connect the {props.role} wallet to use these buttons.</p>;
}

function VaultActions(props: Props & { roles: Roles }) {
  const { contracts: c, address, vault, roles } = props;
  const tx = useTx(address, props.onTxDone);
  const isManager = address === roles.vaultManager;
  const canRebalance = isManager || address === roles.rebalanceManager;
  const sym = vault?.usdc.symbol ?? 'USDC';
  const d = vault?.usdc.decimals ?? 7;
  const invalid = (label: string) => () => tx.setStatus({ kind: 'error', label, message: 'Enter a positive amount.' });
  const locked = tx.busy || !vault;

  return (
    <Card title="Vault · managers" aside={<AddressLink address={c.vault} />}>
      <RoleRow label="Manager" holder={roles.vaultManager} address={address} />
      <RoleRow label="Rebalance manager" holder={roles.rebalanceManager} address={address} />
      <RoleRow label="Emergency manager" holder={roles.emergencyManager} address={address} />
      <RoleRow label="Fee receiver (partner)" holder={roles.feeReceiver} address={address} />
      <NotHolder show={!canRebalance} role="vault manager" />
      {vault && (
        <p className="muted small">
          Idle in the vault: {formatUnits(vault.idle, d)} {sym} · in the strategy: {formatUnits(vault.strategyBalance, d)} {sym}
        </p>
      )}
      <div className="actions">
        <AmountAction
          title="Invest into the strategy"
          hint="rebalance: Invest. Moves idle vault USDC into the strategy."
          unit={sym}
          decimals={d}
          button="Invest"
          disabled={locked || !canRebalance}
          onInvalid={invalid('Invest')}
          onSubmit={(amount) =>
            void tx.run('Invest into the strategy', c.vault, 'rebalance', {
              caller: address,
              instructions: [{ tag: 'Invest', values: [c.strategy, amount] }],
            })
          }
        />
        <AmountAction
          title="Unwind from the strategy"
          hint="rebalance: Unwind. Pulls USDC back from the strategy to the vault."
          unit={sym}
          decimals={d}
          button="Unwind"
          disabled={locked || !canRebalance}
          onInvalid={invalid('Unwind')}
          onSubmit={(amount) =>
            void tx.run('Unwind from the strategy', c.vault, 'rebalance', {
              caller: address,
              instructions: [{ tag: 'Unwind', values: [c.strategy, amount] }],
            })
          }
        />
        <Action title="Revenue share" hint="report books gains, lock_fees takes the vault fee, distribute_fees pays it out.">
          <div className="btn-row">
            <button className="btn btn-small" disabled={tx.busy || !isManager} onClick={() => void tx.run('Report', c.vault, 'report', {})}>
              1. Report
            </button>
            <button
              className="btn btn-small"
              disabled={tx.busy || !isManager}
              onClick={() => void tx.run('Lock fees', c.vault, 'lock_fees', { new_fee_bps: undefined })}
            >
              2. Lock fees
            </button>
            <button
              className="btn btn-small"
              disabled={tx.busy || !isManager}
              onClick={() => void tx.run('Distribute fees', c.vault, 'distribute_fees', { caller: address })}
            >
              3. Distribute
            </button>
          </div>
        </Action>
      </div>
      <TxNotice status={tx.status} />
    </Card>
  );
}

function StrategyActions(props: Props & { roles: Roles }) {
  const { contracts: c, address, strategy: s, roles } = props;
  const tx = useTx(address, props.onTxDone);
  const isKeeper = address === roles.strategyKeeper;
  const canPause = isKeeper || address === roles.strategyAdmin;
  const sym = s?.usdcSymbol ?? 'USDC';
  const d = s?.assetDecimals ?? 7;
  const invalid = (label: string) => () => tx.setStatus({ kind: 'error', label, message: 'Enter a positive amount.' });
  const locked = tx.busy || !s;

  return (
    <Card title="Strategy · keeper and admin" aside={<AddressLink address={c.strategy} />}>
      <RoleRow label="Admin" holder={roles.strategyAdmin} address={address} />
      <RoleRow label="Keeper" holder={roles.strategyKeeper} address={address} />
      <NotHolder show={!canPause} role="strategy keeper or admin" />
      {s && (
        <p className="muted small">
          Idle: {formatUnits(s.idleUsdc, d)} {sym} · RWA held: {formatUnits(s.rwaHeld, s.rwaDecimals)} {s.rwaSymbol} · max swap{' '}
          {formatUnits(s.maxSwap, d, 0)} {sym}
        </p>
      )}
      <div className="actions">
        <AmountAction
          title="Buy RWA (invest)"
          hint="Keeper. Swaps idle strategy USDC for the RWA on the pool. Needs Live pricing."
          unit={sym}
          decimals={d}
          button="Invest"
          disabled={locked || !isKeeper}
          onInvalid={invalid('Invest')}
          onSubmit={(amount) => void tx.run('Keeper invest', c.strategy, 'invest', { usdc_amount: amount })}
        />
        <AmountAction
          title="Sell RWA (divest)"
          hint="Keeper. Sells RWA on the pool to raise this much idle USDC."
          unit={sym}
          decimals={d}
          button="Divest"
          disabled={locked || !isKeeper}
          onInvalid={invalid('Divest')}
          onSubmit={(amount) => void tx.run('Keeper divest', c.strategy, 'divest', { usdc_target: amount })}
        />
        <Action title="Deposits" hint="Admin or keeper. Pausing stops deposit and invest; withdrawals keep working.">
          <div className="btn-row">
            <span className="muted small">{s ? (s.paused ? 'Paused' : 'Open') : '…'}</span>
            <button
              className="btn btn-small"
              disabled={locked || !canPause}
              onClick={() =>
                void tx.run(s?.paused ? 'Unpause deposits' : 'Pause deposits', c.strategy, 'pause_deposits', {
                  caller: address,
                  paused: !s?.paused,
                })
              }
            >
              {s?.paused ? 'Unpause' : 'Pause'}
            </button>
          </div>
        </Action>
      </div>
      <TxNotice status={tx.status} />
    </Card>
  );
}

function formatSeconds(ts: bigint): string {
  return new Date(Number(ts) * 1000).toISOString().replace('T', ' ').slice(0, 16) + ' UTC';
}

function OracleAndTokenActions(props: Props & { roles: Roles }) {
  const { contracts: c, address, vault, roles } = props;
  const tx = useTx(address, props.onTxDone);
  const isOracleAdmin = address === roles.oracleAdmin;
  const isTokenAdmin = address === roles.tokenAdmin;
  const od = roles.oracleDecimals;
  const nav = roles.oracleNav;
  // No push at all yet: fall back to the strategy's anchor, rescaled from the rate's
  // asset_decimals + 12 decimals (see formatRate) to the oracle's.
  const s = props.strategy;
  const rateDecimals = s ? s.assetDecimals + 12 : 0;
  const fallbackNav =
    !s || s.anchor === null
      ? null
      : rateDecimals >= od
        ? s.anchor / 10n ** BigInt(rateDecimals - od)
        : s.anchor * 10n ** BigInt(od - rateDecimals);
  const navToPush = nav?.price ?? fallbackNav;
  const sym = vault?.usdc.symbol ?? 'USDC';
  const d = vault?.usdc.decimals ?? 7;
  const [mintTo, setMintTo] = useState('');
  const [mintAmount, setMintAmount] = useState('');

  function mint(): void {
    const amount = parseUnits(mintAmount, d);
    if (amount === null || amount <= 0n) {
      tx.setStatus({ kind: 'error', label: 'Mint', message: 'Enter a positive amount.' });
      return;
    }
    void tx.run(`Mint ${sym}`, c.usdc, 'mint', { to: mintTo.trim() || address, amount });
  }

  function pushNav(price: bigint): void {
    // Reflector keeps one price per resolution slot, keyed by the slot's start in ms.
    const res = BigInt(roles.oracleResolutionMs);
    const slot = (BigInt(Date.now()) / res) * res;
    void tx.run('Push NAV', c.oracle, 'set_price', { updates: [price], timestamp: slot });
  }

  return (
    <Card title="Oracle and test token" aside={<AddressLink address={c.oracle} />}>
      <RoleRow label="Oracle admin" holder={roles.oracleAdmin} address={address} />
      <RoleRow label="Test token admin" holder={roles.tokenAdmin} address={address} />
      <Row label="Oracle NAV">
        {nav ? `${formatUnits(nav.price, od, 6)} ${sym} · slot ${formatSeconds(nav.timestamp)} ` : 'none '}
        <span className={`badge ${roles.oracleLive ? 'badge-live' : 'badge-oracledown'}`}>{roles.oracleLive ? 'live' : 'expired'}</span>
      </Row>
      <p className="muted small">
        The oracle stops answering an hour after the last push, which puts the strategy in OracleDown. Push at least hourly.
      </p>
      <NotHolder show={!isOracleAdmin && !isTokenAdmin} role="oracle or token admin" />
      <div className="actions">
        <Action
          title="Keep the oracle alive"
          hint="Re-pushes the last NAV into the current slot, which also revives an expired oracle."
        >
          <button className="btn btn-small" disabled={tx.busy || !isOracleAdmin || navToPush === null} onClick={() => navToPush !== null && pushNav(navToPush)}>
            Re-push {navToPush !== null ? formatUnits(navToPush, od, 6) : ''} {sym}
          </button>
        </Action>
        <AmountAction
          title="Set a new NAV"
          hint="USDC per RWA. Raising it is how yield accrues in the demo."
          unit={`NAV in ${sym}`}
          decimals={od}
          button="Push"
          initial={navToPush !== null ? formatUnits(navToPush, od, 6).replace(/,/g, '') : undefined}
          disabled={tx.busy || !isOracleAdmin}
          onInvalid={() => tx.setStatus({ kind: 'error', label: 'Push NAV', message: 'Enter a positive NAV.' })}
          onSubmit={pushNav}
        />
        <Action title={`Mint test ${sym}`} hint="Token admin. Sends test USDC to any address; empty means your wallet.">
          <div className="input-row">
            <input placeholder="G… or C… address" value={mintTo} onChange={(e) => setMintTo(e.target.value)} />
          </div>
          <div className="input-row">
            <input inputMode="decimal" placeholder={sym} value={mintAmount} onChange={(e) => setMintAmount(e.target.value)} />
            <button className="btn btn-small" disabled={tx.busy || !isTokenAdmin} onClick={mint}>
              Mint
            </button>
          </div>
        </Action>
      </div>
      <TxNotice status={tx.status} />
    </Card>
  );
}

export function AdminPanel(props: Props) {
  const [roles, setRoles] = useState<Roles | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { contracts, onTxDone } = props;

  const reload = useCallback((): void => {
    loadRoles(contracts)
      .then((r) => {
        setRoles(r);
        setError(null);
      })
      .catch((err: unknown) => setError(errorMessage(err)));
  }, [contracts]);

  useEffect(reload, [reload]);

  const afterTx = useCallback((): void => {
    reload();
    onTxDone();
  }, [reload, onTxDone]);

  if (!roles) {
    return (
      <Card title="Admin" error={error}>
        <p className="muted">{error ? 'Could not read the roles.' : 'Loading roles…'}</p>
      </Card>
    );
  }
  const shared = { ...props, roles, onTxDone: afterTx };
  return (
    <>
      {!props.address && <p className="warn">Connect the admin, manager or keeper wallet to send these transactions.</p>}
      <div className="grid">
      <VaultActions {...shared} />
      <StrategyActions {...shared} />
      <OracleAndTokenActions {...shared} />
      </div>
    </>
  );
}
