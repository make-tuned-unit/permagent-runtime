/**
 * Finance — The Financier's ledger. Watchlist with live quotes, research
 * notes, and recorded positions. The Financier writes the same rows through
 * its tools; this tab is the place you inspect and edit them by hand.
 * Research, not a brokerage: nothing here places an order.
 */

import { useCallback, useEffect, useState } from 'react';
import type { CSSProperties, FormEvent } from 'react';
import { font, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { ViewHeader } from '../common/ViewHeader';
import { navigateToTool } from '../../lib/store';

interface Quote {
  symbol: string;
  name?: string | null;
  currency?: string | null;
  price?: number | null;
  change?: number | null;
  changePercent?: number | null;
  dayHigh?: number | null;
  dayLow?: number | null;
  fiftyTwoWeekHigh?: number | null;
  fiftyTwoWeekLow?: number | null;
  volume?: number | null;
  quotedAt?: string | null;
  marketClosed: boolean;
}

interface WatchlistRow {
  id: string;
  symbol: string;
  label?: string | null;
  notes?: string | null;
  quote?: Quote | null;
  quoteError?: string | null;
}

interface FinanceNote {
  id: string;
  title: string;
  body: string;
  symbol?: string | null;
  createdAt: string;
  updatedAt: string;
}

interface Position {
  id: string;
  symbol: string;
  companyName: string;
  entryDate: string;
  entryPrice: number;
  shares: number;
  exitDate?: string | null;
  exitPrice?: number | null;
  notes?: string | null;
}

interface PickerStatus {
  reachable: boolean;
  baseUrl: string;
  scanInProgress: boolean;
  scanDate?: string | null;
  results?: number | null;
  detail?: string | null;
}

interface FinanceBoard {
  watchlist: WatchlistRow[];
  notes: FinanceNote[];
  positions: Position[];
  picker: PickerStatus;
}

const POLL_MS = 12_000;

function fmtPrice(n: number | null | undefined, currency?: string | null): string {
  if (n == null || Number.isNaN(n)) return '—';
  const c = currency && currency.length === 3 ? currency : '';
  return `${c ? `${c} ` : ''}${n.toFixed(2)}`;
}

function fmtPct(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '';
  const sign = n > 0 ? '+' : '';
  return `${sign}${n.toFixed(2)}%`;
}

export function FinanceView() {
  const { colors } = useTheme();
  const [board, setBoard] = useState<FinanceBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const next = await apiFetch<FinanceBoard>('/api/finance');
      setBoard(next);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load the Finance tab');
    }
  }, []);

  useEffect(() => {
    void load();
    const t = setInterval(() => { void load(); }, POLL_MS);
    return () => clearInterval(t);
  }, [load]);

  const mutate = useCallback(async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Update failed');
    } finally {
      setBusy(false);
    }
  }, [load]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', fontFamily: font.body, color: colors.text }}>
      <ViewHeader
        title="Finance"
        subtitle="The Financier's ledger — quotes, notes, positions. Research, not advice."
        actions={
          <button type="button" onClick={() => { void load(); }} style={ghostBtn(colors)}>
            Refresh quotes
          </button>
        }
      />
      <div style={{ flex: 1, overflow: 'auto', padding: '20px 24px 40px', display: 'flex', flexDirection: 'column', gap: 28 }}>
        {error && (
          <div style={{ ...type.micro, color: colors.danger }}>{error}</div>
        )}
        {!board && !error && (
          <div style={{ ...type.micro, color: colors.textMuted }}>Loading the ledger…</div>
        )}
        {board && (
          <>
            <WatchlistSection board={board} colors={colors} busy={busy} mutate={mutate} />
            <PositionsSection board={board} colors={colors} busy={busy} mutate={mutate} />
            <NotesSection board={board} colors={colors} busy={busy} mutate={mutate} />
            <PickerSection picker={board.picker} colors={colors} />
          </>
        )}
      </div>
    </div>
  );
}

function WatchlistSection({
  board, colors, busy, mutate,
}: {
  board: FinanceBoard;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
}) {
  const [symbol, setSymbol] = useState('');
  const [label, setLabel] = useState('');

  const onAdd = (e: FormEvent) => {
    e.preventDefault();
    const s = symbol.trim();
    if (!s) return;
    void mutate(async () => {
      await apiFetch('/api/finance/watchlist', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ symbol: s, label: label.trim() || undefined }),
      });
      setSymbol('');
      setLabel('');
    });
  };

  return (
    <section>
      <SectionTitle colors={colors}>Watchlist</SectionTitle>
      {board.watchlist.length === 0 && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 12px' }}>
          Empty. Add a ticker, or ask The Financier to put one on the board.
        </p>
      )}
      {board.watchlist.length > 0 && (
        <table style={tableStyle(colors)}>
          <thead>
            <tr>
              <th style={th(colors)}>Symbol</th>
              <th style={th(colors)}>Price</th>
              <th style={th(colors)}>Day</th>
              <th style={th(colors)}>52-week</th>
              <th style={th(colors)} />
            </tr>
          </thead>
          <tbody>
            {board.watchlist.map((row) => {
              const q = row.quote;
              const up = (q?.change ?? 0) > 0;
              const down = (q?.change ?? 0) < 0;
              return (
                <tr key={row.id}>
                  <td style={td(colors)}>
                    <div style={{ fontWeight: 600 }}>{row.symbol}</div>
                    <div style={{ ...type.micro, color: colors.textMuted }}>
                      {row.label || q?.name || (q?.marketClosed ? 'previous close' : '')}
                    </div>
                  </td>
                  <td style={td(colors)}>
                    {row.quoteError ? (
                      <span style={{ color: colors.textMuted }}>{row.quoteError}</span>
                    ) : (
                      <>
                        <div>{fmtPrice(q?.price, q?.currency)}</div>
                        <div style={{ ...type.micro, color: up ? colors.cyan : down ? colors.danger : colors.textMuted }}>
                          {fmtPct(q?.changePercent)}
                        </div>
                      </>
                    )}
                  </td>
                  <td style={td(colors)}>
                    {q ? `${fmtPrice(q.dayLow)} – ${fmtPrice(q.dayHigh)}` : '—'}
                  </td>
                  <td style={td(colors)}>
                    {q ? `${fmtPrice(q.fiftyTwoWeekLow)} – ${fmtPrice(q.fiftyTwoWeekHigh)}` : '—'}
                  </td>
                  <td style={td(colors)}>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void mutate(() =>
                        apiFetch(`/api/finance/watchlist/${encodeURIComponent(row.symbol)}`, { method: 'DELETE' }),
                      )}
                      style={ghostBtn(colors)}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <form onSubmit={onAdd} style={{ display: 'flex', gap: 8, marginTop: 12, flexWrap: 'wrap' }}>
        <input value={symbol} onChange={(e) => setSymbol(e.target.value)} placeholder="AAPL" style={inputStyle(colors)} />
        <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Name (optional)" style={inputStyle(colors)} />
        <button type="submit" disabled={busy || !symbol.trim()} style={ghostBtn(colors)}>Add</button>
      </form>
    </section>
  );
}

function PositionsSection({
  board, colors, busy, mutate,
}: {
  board: FinanceBoard;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
}) {
  const [symbol, setSymbol] = useState('');
  const [company, setCompany] = useState('');
  const [date, setDate] = useState('');
  const [price, setPrice] = useState('');
  const [shares, setShares] = useState('');

  const onAdd = (e: FormEvent) => {
    e.preventDefault();
    void mutate(async () => {
      await apiFetch('/api/finance/positions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          symbol,
          companyName: company,
          entryDate: date,
          entryPrice: Number(price),
          shares: Number(shares),
        }),
      });
      setSymbol('');
      setCompany('');
      setDate('');
      setPrice('');
      setShares('');
    });
  };

  return (
    <section>
      <SectionTitle colors={colors}>Positions</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 12px' }}>
        Trades you already made. Recording here does not buy or sell anything.
      </p>
      {board.positions.length > 0 && (
        <table style={tableStyle(colors)}>
          <thead>
            <tr>
              <th style={th(colors)}>Symbol</th>
              <th style={th(colors)}>Entry</th>
              <th style={th(colors)}>Shares</th>
              <th style={th(colors)}>Exit</th>
              <th style={th(colors)} />
            </tr>
          </thead>
          <tbody>
            {board.positions.map((p) => (
              <tr key={p.id}>
                <td style={td(colors)}>
                  <div style={{ fontWeight: 600 }}>{p.symbol}</div>
                  <div style={{ ...type.micro, color: colors.textMuted }}>{p.companyName}</div>
                </td>
                <td style={td(colors)}>{p.entryDate} · {fmtPrice(p.entryPrice)}</td>
                <td style={td(colors)}>{p.shares}</td>
                <td style={td(colors)}>
                  {p.exitDate ? `${p.exitDate} · ${fmtPrice(p.exitPrice)}` : 'open'}
                </td>
                <td style={td(colors)}>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void mutate(() =>
                      apiFetch(`/api/finance/positions/${encodeURIComponent(p.id)}`, { method: 'DELETE' }),
                    )}
                    style={ghostBtn(colors)}
                  >
                    Remove
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <form onSubmit={onAdd} style={{ display: 'flex', gap: 8, marginTop: 12, flexWrap: 'wrap' }}>
        <input value={symbol} onChange={(e) => setSymbol(e.target.value)} placeholder="Ticker" style={inputStyle(colors)} required />
        <input value={company} onChange={(e) => setCompany(e.target.value)} placeholder="Company" style={inputStyle(colors)} required />
        <input value={date} onChange={(e) => setDate(e.target.value)} placeholder="YYYY-MM-DD" style={inputStyle(colors)} required />
        <input value={price} onChange={(e) => setPrice(e.target.value)} placeholder="Entry price" style={inputStyle(colors)} required />
        <input value={shares} onChange={(e) => setShares(e.target.value)} placeholder="Shares" style={inputStyle(colors)} required />
        <button type="submit" disabled={busy} style={ghostBtn(colors)}>Record</button>
      </form>
    </section>
  );
}

function NotesSection({
  board, colors, busy, mutate,
}: {
  board: FinanceBoard;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
}) {
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [symbol, setSymbol] = useState('');

  const onAdd = (e: FormEvent) => {
    e.preventDefault();
    void mutate(async () => {
      await apiFetch('/api/finance/notes', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title, body, symbol: symbol.trim() || undefined }),
      });
      setTitle('');
      setBody('');
      setSymbol('');
    });
  };

  return (
    <section>
      <SectionTitle colors={colors}>Notes</SectionTitle>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        {board.notes.map((n) => (
          <div key={n.id} style={{ border: `1px solid ${colors.border}`, padding: '12px 14px', borderRadius: 6 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, alignItems: 'baseline' }}>
              <div>
                <span style={{ fontWeight: 600 }}>{n.title}</span>
                {n.symbol && (
                  <span style={{ ...type.micro, color: colors.textMuted, marginLeft: 8 }}>{n.symbol}</span>
                )}
              </div>
              <button
                type="button"
                disabled={busy}
                onClick={() => void mutate(() =>
                  apiFetch(`/api/finance/notes/${encodeURIComponent(n.id)}`, { method: 'DELETE' }),
                )}
                style={ghostBtn(colors)}
              >
                Delete
              </button>
            </div>
            <p style={{ ...type.body, margin: '8px 0 0', whiteSpace: 'pre-wrap' }}>{n.body}</p>
          </div>
        ))}
      </div>
      <form onSubmit={onAdd} style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 12 }}>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" style={inputStyle(colors)} required />
          <input value={symbol} onChange={(e) => setSymbol(e.target.value)} placeholder="Ticker (optional)" style={inputStyle(colors)} />
        </div>
        <textarea value={body} onChange={(e) => setBody(e.target.value)} placeholder="Observation, sourced number, or question — not a recommendation." style={{ ...inputStyle(colors), minHeight: 72 }} required />
        <button type="submit" disabled={busy} style={{ ...ghostBtn(colors), alignSelf: 'flex-start' }}>Add note</button>
      </form>
    </section>
  );
}

function PickerSection({ picker, colors }: { picker: PickerStatus; colors: ThemeColors }) {
  return (
    <section>
      <SectionTitle colors={colors}>Scanner</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>
        {picker.reachable
          ? `Your stock scanner is up at ${picker.baseUrl}${picker.scanInProgress ? ' — a scan is running' : picker.scanDate ? ` — last scan ${picker.scanDate}` : ''}. The Financier can start a scan and read picks; this tab does not re-rank them.`
          : `Scanner not reachable${picker.detail ? ` (${picker.detail})` : ''}. Quotes and the ledger still work.`}
      </p>
      <button
        type="button"
        onClick={() => navigateToTool('world')}
        style={{ ...ghostBtn(colors), marginTop: 10 }}
      >
        Open The Financier in World
      </button>
    </section>
  );
}

function SectionTitle({ children, colors }: { children: string; colors: ThemeColors }) {
  return (
    <h2 style={{ ...type.micro, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textMuted, margin: '0 0 10px' }}>
      {children}
    </h2>
  );
}

function tableStyle(colors: ThemeColors): CSSProperties {
  return { width: '100%', borderCollapse: 'collapse', fontFamily: font.body };
}
function th(colors: ThemeColors): CSSProperties {
  return { ...type.micro, textAlign: 'left', color: colors.textMuted, padding: '6px 10px 6px 0', borderBottom: `1px solid ${colors.border}`, fontWeight: 600 };
}
function td(colors: ThemeColors): CSSProperties {
  return { ...type.body, padding: '10px 10px 10px 0', borderBottom: `1px solid ${colors.border}`, verticalAlign: 'top' };
}
function inputStyle(colors: ThemeColors): CSSProperties {
  return {
    ...type.body,
    background: colors.bg,
    color: colors.text,
    border: `1px solid ${colors.border}`,
    borderRadius: 4,
    padding: '6px 10px',
    fontFamily: font.body,
    minWidth: 120,
  };
}
function ghostBtn(colors: ThemeColors): CSSProperties {
  return {
    ...type.micro,
    background: 'transparent',
    color: colors.text,
    border: `1px solid ${colors.border}`,
    borderRadius: 4,
    padding: '6px 10px',
    cursor: 'pointer',
    fontFamily: font.body,
  };
}
