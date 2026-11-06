// Test tokens for reviewers: testnet XLM from Friendbot, then TUSDC and TBOND from our faucet.

import { useCallback, useEffect, useState } from 'react';
import { invoke, simulate } from './chain';
import type { Contracts } from './config';
import { loadToken } from './data';
import { errorMessage, formatUnits } from './format';
import { AddressLink, Card, TxNotice, type TxStatus } from './ui';
import { signerFor } from './wallet';

interface FaucetInfo {
  lines: string[];
  cooldown: bigint;
  /** Unix seconds from which the connected address can claim again. */
  nextClaim: bigint;
}

async function loadFaucet(faucet: string, address: string): Promise<FaucetInfo> {
  const [[tokens, amounts, cooldown], nextClaim] = await Promise.all([
    simulate<[string[], bigint[], bigint]>(faucet, 'config'),
    simulate<bigint>(faucet, 'next_claim', { to: address }),
  ]);
  const infos = await Promise.all(tokens.map((t) => loadToken(t)));
  const lines = infos.map((t, i) => `${formatUnits(amounts[i], t.decimals, 0)} ${t.symbol}`);
  return { lines, cooldown, nextClaim };
}

export function FaucetCard(props: { contracts: Contracts; address: string | null; accountExists: boolean | null; onTxDone: () => void }) {
  const { contracts, address, onTxDone } = props;
  const faucet = contracts.faucet;
  const [info, setInfo] = useState<FaucetInfo | null>(null);
  const [tx, setTx] = useState<TxStatus | null>(null);
  const busy = tx?.kind === 'pending';

  const reload = useCallback((): void => {
    if (!faucet || !address) return;
    loadFaucet(faucet, address).then(setInfo, (err: unknown) => setTx({ kind: 'error', label: 'Faucet', message: errorMessage(err) }));
  }, [faucet, address]);
  useEffect(reload, [reload]);

  if (!faucet) return null;

  async function claim(): Promise<void> {
    if (!address || !faucet) return;
    setTx({ kind: 'pending', label: 'Claim test tokens' });
    try {
      const hash = await invoke(faucet, 'claim', { to: address }, address, signerFor(address));
      setTx({ kind: 'ok', label: 'Test tokens received', hash });
      reload();
      onTxDone();
    } catch (err) {
      setTx({ kind: 'error', label: 'Claim failed', message: errorMessage(err) });
    }
  }

  const now = BigInt(Math.floor(Date.now() / 1000));
  const waiting = info !== null && info.nextClaim > now;

  return (
    <Card title="Get test tokens" aside={<AddressLink address={faucet} />}>
      <ol className="steps">
        <li>
          Fund your account with testnet XLM for fees:{' '}
          {address ? (
            <a href={`https://friendbot.stellar.org/?addr=${address}`} target="_blank" rel="noreferrer">
              Friendbot
            </a>
          ) : (
            'connect a wallet first'
          )}
          {props.accountExists === true && <span className="muted small"> (done)</span>}
        </li>
        <li>
          Claim {info ? info.lines.join(' and ') : 'test TUSDC and TBOND'} from the faucet, once an hour.
        </li>
        <li>Deposit TUSDC into the vault in "My position".</li>
      </ol>
      <button className="btn" onClick={() => void claim()} disabled={!address || busy || waiting || props.accountExists === false}>
        {waiting ? `Next claim at ${new Date(Number(info!.nextClaim) * 1000).toLocaleTimeString()}` : 'Claim test tokens'}
      </button>
      <TxNotice status={tx} />
      <p className="muted small">
        TBOND is the test RWA the strategy buys. You don't need it to deposit; it's there to trade on the pool. To see the
        tokens in your wallet, add them by contract ID: TUSDC <AddressLink address={contracts.usdc} />, TBOND{' '}
        <AddressLink address={contracts.rwa} />.
      </p>
    </Card>
  );
}
