import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { CONTRACTS, CONTRACTS_PROBLEM, REFRESH_MS, type Contracts } from './config';
import {
  loadPosition,
  loadStrategy,
  loadVault,
  type Position,
  type StrategyState,
  type VaultState,
} from './data';
import { errorMessage, shortAddress } from './format';
import { PositionCard } from './PositionCard';
import { StrategyCard } from './StrategyCard';
import { VaultCard } from './VaultCard';
import { connectWallet, disconnectWallet } from './wallet';

const TITLE = 'The Strategists · RWA Yield-Sharing (Testnet)';

function Header(props: { address: string | null; onConnect: () => void; onDisconnect: () => void }) {
  return (
    <header className="top">
      <div>
        <h1>{TITLE}</h1>
        <p className="muted">A vault on Stellar that earns from one tokenized real-world-asset strategy.</p>
      </div>
      <div className="top-actions">
        <span className="badge badge-net">Testnet</span>
        {props.address ? (
          <button className="btn btn-ghost" onClick={props.onDisconnect} title={props.address}>
            {shortAddress(props.address)} · Disconnect
          </button>
        ) : (
          <button className="btn" onClick={props.onConnect}>
            Connect wallet
          </button>
        )}
      </div>
    </header>
  );
}

function NotDeployed() {
  return (
    <section className="card">
      <h2>Not deployed yet</h2>
      <p>The testnet contracts are not deployed yet, so there is nothing to show.</p>
      <p className="muted small">{CONTRACTS_PROBLEM}</p>
    </section>
  );
}

interface CardState<T> {
  data: T | null;
  error: string | null;
}

const EMPTY_CARD = { data: null, error: null };

/** Runs `load` and stores its result, or its error next to the last good value. */
async function loadInto<T>(load: () => Promise<T>, set: Dispatch<SetStateAction<CardState<T>>>): Promise<void> {
  try {
    const data = await load();
    set({ data, error: null });
  } catch (err) {
    set((prev) => ({ data: prev.data, error: errorMessage(err) }));
  }
}

function Dashboard(props: { contracts: Contracts; address: string | null }) {
  const { contracts, address } = props;
  const [vault, setVault] = useState<CardState<VaultState>>(EMPTY_CARD);
  const [strategy, setStrategy] = useState<CardState<StrategyState>>(EMPTY_CARD);
  const [position, setPosition] = useState<CardState<Position>>(EMPTY_CARD);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const inFlight = useRef<{ address: string | null; promise: Promise<void> } | null>(null);
  const addressRef = useRef(address);
  addressRef.current = address;

  const refresh = useCallback((): Promise<void> => {
    // One refresh at a time. A testnet read can take several seconds, so a fixed interval
    // used to start new rounds before the last one finished and the RPC queue only grew.
    if (inFlight.current && inFlight.current.address === address) return inFlight.current.promise;
    const forAddress = address;
    // Guards against a slow read for a previous wallet landing after a switch.
    const setPositionIfCurrent: Dispatch<SetStateAction<CardState<Position>>> = (v) => {
      if (addressRef.current === forAddress) setPosition(v);
    };
    // Each card renders as soon as its own reads finish.
    const round = Promise.all([
      loadInto(() => loadVault(contracts), setVault),
      loadInto(() => loadStrategy(contracts), setStrategy),
      forAddress ? loadInto(() => loadPosition(contracts, forAddress), setPositionIfCurrent) : Promise.resolve(),
    ]).then(() => setUpdatedAt(new Date()));
    const promise = round.finally(() => {
      if (inFlight.current?.promise === promise) inFlight.current = null;
    });
    inFlight.current = { address: forAddress, promise };
    return promise;
  }, [contracts, address]);

  useEffect(() => {
    setPosition(EMPTY_CARD);
    let cancelled = false;
    let timer: number | undefined;
    // The next round is scheduled only after this one settles, never on a fixed clock.
    const tick = async (): Promise<void> => {
      await refresh();
      if (!cancelled) timer = window.setTimeout(() => void tick(), REFRESH_MS);
    };
    void tick();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [refresh]);

  return (
    <>
      <p className="muted small updated">
        {updatedAt ? `Updated ${updatedAt.toLocaleTimeString()} · refreshes every ${REFRESH_MS / 1000}s` : 'Loading from testnet…'}{' '}
        <button className="link" onClick={() => void refresh()}>
          Refresh now
        </button>
      </p>
      <div className="grid">
        <VaultCard contracts={contracts} vault={vault.data} error={vault.error} />
        <StrategyCard contracts={contracts} strategy={strategy.data} error={strategy.error} />
        <PositionCard
          contracts={contracts}
          address={address}
          vault={vault.data}
          position={position.data}
          error={position.error}
          onTxDone={() => void refresh()}
        />
      </div>
    </>
  );
}

export default function App() {
  const [address, setAddress] = useState<string | null>(null);
  const [walletError, setWalletError] = useState<string | null>(null);

  async function connect(): Promise<void> {
    setWalletError(null);
    try {
      setAddress(await connectWallet());
    } catch (err) {
      setWalletError(errorMessage(err));
    }
  }

  async function disconnect(): Promise<void> {
    await disconnectWallet().catch(() => undefined);
    setAddress(null);
  }

  return (
    <main className="page">
      <Header address={address} onConnect={() => void connect()} onDisconnect={() => void disconnect()} />
      {walletError && <p className="error">{walletError}</p>}
      {CONTRACTS ? <Dashboard contracts={CONTRACTS} address={address} /> : <NotDeployed />}
      <footer className="muted small">Stellar testnet · test funds only, no real value.</footer>
    </main>
  );
}
