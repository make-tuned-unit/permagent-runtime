/**
 * Finance — money board. Polybot status, holdings with live P&L, Picker
 * picks run through Yahoo + a loop-engineering gate, overbought sell
 * signals on open holdings, household spend, then the research ledger. Research, not a
 * brokerage: nothing here places an order. The Picker ranker never sees
 * holdings or bank balances.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, FormEvent, ReactNode } from 'react';
import { font, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch, uploadFinanceStatement } from '../../lib/api';
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

interface PolybotStatus {
  found: boolean;
  root?: string | null;
  paused: boolean;
  currentBalance?: number | null;
  realizedPnl?: number | null;
  openExposure?: number | null;
  tradeCount?: number | null;
  lastUpdated?: string | null;
  stale: boolean;
  detail?: string | null;
}

interface HoldingRow {
  id: string;
  symbol: string;
  companyName: string;
  entryDate: string;
  entryPrice: number;
  shares: number;
  exitDate?: string | null;
  exitPrice?: number | null;
  notes?: string | null;
  source: string;
  quote?: Quote | null;
  quoteError?: string | null;
  last?: number | null;
  unrealized?: number | null;
  unrealizedPct?: number | null;
  realized?: number | null;
  rsi?: number | null;
  sellSignal?: boolean;
  overboughtSigns?: string[];
}

interface HoldingsView {
  source: string;
  openCount: number;
  netUnrealized: number;
  netRealized: number;
  netPnl: number;
  rows: HoldingRow[];
}

interface LoopGate {
  icir?: number | null;
  icMean?: number | null;
  halfLifeDays?: number | null;
  oosIcir?: number | null;
  passed: boolean;
  kills: string[];
  batchSize: number;
}

interface FundamentalsView {
  available: boolean;
  summary?: string | null;
  error?: string | null;
}

interface ValidatedPick {
  ticker: string;
  companyName?: string | null;
  rank?: number | null;
  score?: number | null;
  tier?: string | null;
  pickerRsi?: number | null;
  pickerPrice?: number | null;
  confidence?: number | null;
  buyWindow?: string | null;
  reason?: string | null;
  quote?: Quote | null;
  quoteError?: string | null;
  priceMismatch: boolean;
  fundamentals: FundamentalsView;
  loop?: LoopGate | null;
}

interface SellSignal {
  symbol: string;
  rsi?: number | null;
  rsiThreshold: number;
  signs: string[];
  summary: string;
}

interface Transaction {
  id: string;
  date: string;
  amount: number;
  payee: string;
  category: string;
  account?: string | null;
  sourceFile?: string | null;
  createdAt: string;
}

interface CategorySpend {
  category: string;
  amount: number;
}

interface Recurring {
  payee: string;
  typicalAmount: number;
  count: number;
}

interface TradeDraft {
  ticker: string;
  company: string;
  date: string;
  price: string;
  shares: string;
  notes: string;
  exitDate: string;
  exitPrice: string;
  editingId: string | null;
  source: string | null;
}

function emptyDraft(): TradeDraft {
  return {
    ticker: '',
    company: '',
    date: '',
    price: '',
    shares: '',
    notes: '',
    exitDate: '',
    exitPrice: '',
    editingId: null,
    source: null,
  };
}

function todayIso(): string {
  return new Date().toISOString().slice(0, 10);
}

function draftFromRow(row: HoldingRow): TradeDraft {
  return {
    ticker: row.symbol,
    company: row.companyName || row.symbol,
    date: row.entryDate,
    price: String(row.entryPrice),
    shares: String(row.shares),
    notes: row.notes ?? '',
    exitDate: row.exitDate ?? '',
    exitPrice: row.exitPrice != null ? String(row.exitPrice) : '',
    editingId: row.id,
    source: row.source,
  };
}

function tradePayload(draft: TradeDraft) {
  const exitDate = draft.exitDate.trim();
  const exitPrice = draft.exitPrice.trim();
  return {
    symbol: draft.ticker,
    companyName: draft.company.trim() || draft.ticker,
    entryDate: draft.date,
    entryPrice: Number(draft.price),
    shares: Number(draft.shares),
    notes: draft.notes.trim() || undefined,
    exitDate: exitDate || undefined,
    exitPrice: exitPrice ? Number(exitPrice) : undefined,
  };
}

interface RecordedTrade {
  local?: Position | null;
  picker?: unknown;
  pickerError?: string | null;
}

interface SpendForecast {
  daysUsed: number;
  spend90d: number;
  runRate30d: number;
  runRate90d: number;
  byCategory: CategorySpend[];
  recurring: Recurring[];
  method: string;
}

interface HouseholdView {
  recent: Transaction[];
  forecast: SpendForecast;
}

interface FinanceBoard {
  polybot: PolybotStatus;
  holdings: HoldingsView;
  watchlist: WatchlistRow[];
  notes: FinanceNote[];
  positions: Position[];
  picker: PickerStatus;
  picks: ValidatedPick[];
  sellSignals: SellSignal[];
  rsiThreshold: number;
  dailyPick?: DailyPick | null;
  household: HouseholdView;
}

interface DailyPick {
  day: string;
  asOf: string;
  ticker?: string | null;
  companyName?: string | null;
  why: string;
  model?: string | null;
  candidateCount: number;
}

const POLL_MS = 60_000;

const CATEGORIES = [
  'housing',
  'groceries',
  'transport',
  'utilities',
  'dining',
  'health',
  'subscriptions',
  'income',
  'transfer',
  'uncategorized',
] as const;

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

function fmtSigned(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '—';
  const sign = n > 0 ? '+' : '';
  return `${sign}${n.toFixed(2)}`;
}

function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

export function FinanceView() {
  const { colors } = useTheme();
  const [board, setBoard] = useState<FinanceBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<TradeDraft>(emptyDraft);

  const load = useCallback(async () => {
    try {
      const next = await apiFetch<FinanceBoard>('/api/finance');
      setBoard(next);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load the Finance tab');
    }
  }, []);

  const scanRunning = Boolean(board?.picker.scanInProgress);
  useEffect(() => {
    void load();
    const t = setInterval(() => { void load(); }, scanRunning ? 10_000 : POLL_MS);
    return () => clearInterval(t);
  }, [load, scanRunning]);

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
        subtitle="Money board — Polybot, holdings, validated picks, household. Research, not advice."
        actions={
          <button type="button" onClick={() => { void load(); }} style={ghostBtn(colors)}>
            Refresh
          </button>
        }
      />
      <div style={{ flex: 1, overflow: 'auto', padding: '20px 24px 40px', display: 'flex', flexDirection: 'column', gap: 28 }}>
        {error && (
          <div style={{ ...type.micro, color: colors.danger }}>{error}</div>
        )}
        {!board && !error && (
          <div style={{ ...type.micro, color: colors.textMuted }}>Loading the money board…</div>
        )}
        {board && (
          <>
            <PolybotSection polybot={board.polybot} colors={colors} />
            <HoldingsSection
              holdings={board.holdings}
              rsiThreshold={board.rsiThreshold}
              picker={board.picker}
              colors={colors}
              busy={busy}
              mutate={mutate}
              draft={draft}
              setDraft={setDraft}
              onRecorded={(hint) => { if (hint) setError(hint); }}
            />
            <TomorrowSection pick={board.dailyPick ?? null} colors={colors} />
            <SellSignalsSection signals={board.sellSignals} threshold={board.rsiThreshold} colors={colors} />
            <PicksSection
              board={board}
              colors={colors}
              busy={busy}
              mutate={mutate}
              onPrefill={(next) => {
                setDraft(next);
                requestAnimationFrame(() => {
                  document.getElementById('finance-holdings-form')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
                });
              }}
            />
            <HouseholdSection
              household={board.household}
              colors={colors}
              busy={busy}
              mutate={mutate}
              setError={setError}
            />
            <WatchlistSection board={board} colors={colors} busy={busy} mutate={mutate} />
            <NotesSection board={board} colors={colors} busy={busy} mutate={mutate} />
          </>
        )}
      </div>
    </div>
  );
}

function PolybotSection({ polybot, colors }: { polybot: PolybotStatus; colors: ThemeColors }) {
  const status = !polybot.found
    ? 'not found'
    : polybot.paused
      ? 'paused'
      : polybot.stale
        ? 'stale'
        : 'live file';
  return (
    <section>
      <SectionTitle colors={colors}>Polybot</SectionTitle>
      <div style={{ display: 'flex', gap: 24, flexWrap: 'wrap', marginBottom: 8 }}>
        <Stat colors={colors} label="Status" value={status} />
        <Stat colors={colors} label="Balance" value={fmtPrice(polybot.currentBalance)} />
        <Stat
          colors={colors}
          label="Realized P&L"
          value={fmtSigned(polybot.realizedPnl)}
          tone={polybot.realizedPnl != null && polybot.realizedPnl < 0 ? 'neg' : undefined}
        />
        <Stat colors={colors} label="Open exposure" value={fmtPrice(polybot.openExposure)} />
        <Stat colors={colors} label="Trades" value={polybot.tradeCount != null ? String(polybot.tradeCount) : '—'} />
      </div>
      <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>
        Read from {polybot.root ? `${polybot.root}/logs/bankroll.json` : 'logs/bankroll.json'}. No CLOB. No vault keys.
        {polybot.lastUpdated ? ` Snapshot ${fmtWhen(polybot.lastUpdated)}.` : ''}
        {polybot.detail ? ` ${polybot.detail}` : ''}
      </p>
    </section>
  );
}

function Stat({
  colors, label, value, tone,
}: {
  colors: ThemeColors;
  label: string;
  value: string;
  tone?: 'neg';
}) {
  return (
    <div>
      <div style={{ ...type.micro, color: colors.textMuted, letterSpacing: '0.06em', textTransform: 'uppercase' }}>{label}</div>
      <div style={{ ...type.body, color: tone === 'neg' ? colors.danger : colors.text, fontWeight: 600 }}>{value}</div>
    </div>
  );
}

function Field({
  colors, label, children, wide,
}: {
  colors: ThemeColors;
  label: string;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: wide ? 220 : 120, flex: wide ? 1 : undefined }}>
      <span style={{ ...type.micro, color: colors.textMuted, letterSpacing: '0.06em', textTransform: 'uppercase' }}>{label}</span>
      {children}
    </label>
  );
}

function PickerControls({
  picker, colors, busy, mutate,
}: {
  picker: PickerStatus;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
}) {
  return (
    <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
      <button
        type="button"
        disabled={busy || picker.reachable}
        onClick={() => void mutate(() =>
          apiFetch('/api/finance/picker/start', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
        )}
        style={ghostBtn(colors)}
      >
        {picker.reachable ? 'Scanner running' : 'Start scanner'}
      </button>
      <button
        type="button"
        disabled={busy || !picker.reachable || picker.scanInProgress}
        onClick={() => void mutate(() =>
          apiFetch('/api/finance/picker/scan', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
        )}
        style={ghostBtn(colors)}
      >
        {picker.scanInProgress ? 'Scan running…' : 'Run scan'}
      </button>
    </div>
  );
}

function HoldingsSection({
  holdings, rsiThreshold, picker, colors, busy, mutate, draft, setDraft, onRecorded,
}: {
  holdings: HoldingsView;
  rsiThreshold: number;
  picker: PickerStatus;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  draft: TradeDraft;
  setDraft: (d: TradeDraft) => void;
  onRecorded: (hint: string | null) => void;
}) {
  const [filter, setFilter] = useState<'open' | 'all'>('all');
  const editing = Boolean(draft.editingId);
  const rows = holdings.rows.filter((r) => filter === 'all' || !r.exitDate);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void mutate(async () => {
      const payload = tradePayload(draft);
      const pickerSourced = draft.source === 'picker';
      const path = draft.editingId
        ? pickerSourced
          ? `/api/finance/picker/trades/${encodeURIComponent(draft.editingId)}`
          : `/api/finance/positions/${encodeURIComponent(draft.editingId)}`
        : '/api/finance/picker/trades';
      const res = await apiFetch<RecordedTrade>(path, {
        method: draft.editingId ? 'PATCH' : 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      setDraft(emptyDraft());
      onRecorded(
        res.pickerError
          ? `Saved on this tab. Scanner history was not updated (${res.pickerError}).`
          : null,
      );
    });
  };

  const onEdit = (row: HoldingRow) => {
    setDraft(draftFromRow(row));
    requestAnimationFrame(() => {
      document.getElementById('finance-holdings-form')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    });
  };

  return (
    <section>
      <SectionTitle colors={colors}>Holdings</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 12px' }}>
        Net P&amp;L {fmtSigned(holdings.netPnl)}
        {' · '}unrealized {fmtSigned(holdings.netUnrealized)}
        {' · '}realized {fmtSigned(holdings.netRealized)}
        {' · '}{holdings.openCount} open
        {' · '}source {holdings.source === 'picker' ? 'Picker trade history' : 'local ledger overlay'}.
        Enter, edit, and close lots here — this is the Picker trade journal. Does not buy or sell, and never feeds the ranker.
        {!picker.reachable ? ' Scanner is down: lots land on this tab until you start it.' : ''}
      </p>
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 14, alignItems: 'center' }}>
        <PickerControls picker={picker} colors={colors} busy={busy} mutate={mutate} />
        <span style={{ ...type.micro, color: colors.textMuted }}>
          {picker.reachable
            ? `${picker.baseUrl}${picker.scanInProgress ? ' — scan running' : picker.scanDate ? ` — last scan ${picker.scanDate}` : ''}${picker.results != null ? ` · ${picker.results} ranked` : ''}`
            : picker.detail || 'scanner not reachable'}
        </span>
      </div>
      <form
        id="finance-holdings-form"
        onSubmit={onSubmit}
        style={{
          border: `1px solid ${colors.border}`,
          borderRadius: 6,
          padding: 14,
          marginBottom: 16,
          display: 'flex',
          flexDirection: 'column',
          gap: 10,
        }}
      >
        <div style={{ ...type.micro, color: colors.textMuted, letterSpacing: '0.06em', textTransform: 'uppercase' }}>
          {editing ? 'Edit trade' : 'Record a trade already made'}
        </div>
        <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
          <Field colors={colors} label="Ticker">
            <input value={draft.ticker} onChange={(e) => setDraft({ ...draft, ticker: e.target.value })} placeholder="SHOP.TO" style={inputStyle(colors)} required />
          </Field>
          <Field colors={colors} label="Company">
            <input value={draft.company} onChange={(e) => setDraft({ ...draft, company: e.target.value })} placeholder="defaults to ticker" style={inputStyle(colors)} />
          </Field>
          <Field colors={colors} label="Entry date">
            <input value={draft.date} onChange={(e) => setDraft({ ...draft, date: e.target.value })} placeholder="YYYY-MM-DD" style={inputStyle(colors)} required />
          </Field>
          <Field colors={colors} label="Entry price">
            <input value={draft.price} onChange={(e) => setDraft({ ...draft, price: e.target.value })} placeholder="0.00" style={inputStyle(colors)} required />
          </Field>
          <Field colors={colors} label="Shares">
            <input value={draft.shares} onChange={(e) => setDraft({ ...draft, shares: e.target.value })} placeholder="0" style={inputStyle(colors)} required />
          </Field>
        </div>
        <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
          <Field colors={colors} label="Exit date (optional)">
            <input value={draft.exitDate} onChange={(e) => setDraft({ ...draft, exitDate: e.target.value })} placeholder="YYYY-MM-DD" style={inputStyle(colors)} />
          </Field>
          <Field colors={colors} label="Exit price (optional)">
            <input value={draft.exitPrice} onChange={(e) => setDraft({ ...draft, exitPrice: e.target.value })} placeholder="0.00" style={inputStyle(colors)} />
          </Field>
          <Field colors={colors} label="Notes" wide>
            <input value={draft.notes} onChange={(e) => setDraft({ ...draft, notes: e.target.value })} placeholder="Why you took it — not an order" style={{ ...inputStyle(colors), minWidth: 220 }} />
          </Field>
        </div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button type="submit" disabled={busy} style={ghostBtn(colors)}>
            {editing ? 'Save trade' : 'Record trade'}
          </button>
          {editing && (
            <button type="button" disabled={busy} onClick={() => setDraft(emptyDraft())} style={ghostBtn(colors)}>
              Cancel
            </button>
          )}
        </div>
      </form>
      {holdings.rows.length === 0 ? (
        <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>
          No lots yet. Record a trade you already made above — you do not need to open Picker.
        </p>
      ) : (
        <>
          <div style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
            <button type="button" onClick={() => setFilter('open')} style={filter === 'open' ? ghostBtnOn(colors) : ghostBtn(colors)}>
              Open ({holdings.openCount})
            </button>
            <button type="button" onClick={() => setFilter('all')} style={filter === 'all' ? ghostBtnOn(colors) : ghostBtn(colors)}>
              All ({holdings.rows.length})
            </button>
          </div>
          <table style={tableStyle(colors)}>
            <thead>
              <tr>
                <th style={th(colors)}>Symbol</th>
                <th style={th(colors)}>Shares</th>
                <th style={th(colors)}>Entry</th>
                <th style={th(colors)}>Mark</th>
                <th style={th(colors)}>P&amp;L</th>
                <th style={th(colors)}>RSI-14</th>
                <th style={th(colors)}>Notes</th>
                <th style={th(colors)} />
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => {
                const closed = Boolean(p.exitDate);
                const rsiHot = Boolean(p.sellSignal) || (!closed && p.rsi != null && p.rsi >= rsiThreshold);
                const pnl = closed ? p.realized : p.unrealized;
                return (
                  <tr key={p.id} style={closed ? { opacity: 0.65 } : undefined}>
                    <td style={td(colors)}>
                      <div style={{ fontWeight: 600 }}>{p.symbol}</div>
                      <div style={{ ...type.micro, color: colors.textMuted }}>{p.companyName || (closed ? 'closed' : 'open')}</div>
                    </td>
                    <td style={td(colors)}>{p.shares}</td>
                    <td style={td(colors)}>{p.entryDate} · {fmtPrice(p.entryPrice)}</td>
                    <td style={td(colors)}>
                      {p.quoteError ? (
                        <span style={{ color: colors.textMuted }}>{p.quoteError}</span>
                      ) : closed ? (
                        `${p.exitDate} · ${fmtPrice(p.exitPrice)}`
                      ) : (
                        fmtPrice(p.last, p.quote?.currency)
                      )}
                    </td>
                    <td style={{ ...td(colors), color: (pnl ?? 0) < 0 ? colors.danger : colors.text }}>
                      {fmtSigned(pnl)}
                      {!closed && p.unrealizedPct != null ? (
                        <div style={{ ...type.micro, color: colors.textMuted }}>{fmtPct(p.unrealizedPct)}</div>
                      ) : null}
                    </td>
                    <td style={{ ...td(colors), color: rsiHot ? colors.danger : colors.text }}>
                      {p.rsi != null ? p.rsi.toFixed(1) : '—'}
                      {p.sellSignal ? (
                        <div style={{ ...type.micro, color: colors.danger }}>sell signal</div>
                      ) : null}
                    </td>
                    <td style={{ ...td(colors), ...type.micro, color: colors.textMuted }}>{p.notes || '—'}</td>
                    <td style={td(colors)}>
                      <LotActions
                        row={p}
                        colors={colors}
                        busy={busy}
                        mutate={mutate}
                        onEdit={() => onEdit(p)}
                      />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}

function LotActions({
  row, colors, busy, mutate, onEdit,
}: {
  row: HoldingRow;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  onEdit: () => void;
}) {
  const [exitDate, setExitDate] = useState(todayIso());
  const [exitPrice, setExitPrice] = useState(row.last != null ? String(row.last) : '');
  const pickerSourced = row.source === 'picker';
  const closePath = pickerSourced
    ? `/api/finance/picker/trades/${encodeURIComponent(row.id)}/close`
    : `/api/finance/positions/${encodeURIComponent(row.id)}/close`;
  const deletePath = pickerSourced
    ? `/api/finance/picker/trades/${encodeURIComponent(row.id)}`
    : `/api/finance/positions/${encodeURIComponent(row.id)}`;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, alignItems: 'flex-start' }}>
      <button type="button" disabled={busy} onClick={onEdit} style={ghostBtn(colors)}>
        Edit
      </button>
      {!row.exitDate && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void mutate(() =>
              apiFetch(closePath, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                  exitDate,
                  exitPrice: Number(exitPrice),
                }),
              }),
            );
          }}
          style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}
        >
          <input value={exitDate} onChange={(e) => setExitDate(e.target.value)} placeholder="Exit YYYY-MM-DD" style={{ ...inputStyle(colors), minWidth: 110 }} required />
          <input value={exitPrice} onChange={(e) => setExitPrice(e.target.value)} placeholder="Exit price" style={{ ...inputStyle(colors), minWidth: 90 }} required />
          <button type="submit" disabled={busy} style={ghostBtn(colors)}>Close</button>
        </form>
      )}
      <button
        type="button"
        disabled={busy}
        onClick={() => void mutate(() => apiFetch(deletePath, { method: 'DELETE' }))}
        style={ghostBtn(colors)}
      >
        Remove
      </button>
    </div>
  );
}


function TomorrowSection({
  pick, colors,
}: {
  pick: DailyPick | null;
  colors: ThemeColors;
}) {
  const ticker = pick?.ticker?.trim() || null;
  return (
    <section>
      <SectionTitle colors={colors}>Tomorrow</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 8px' }}>
        15:30 ET close scan · Opus on names that cleared the loop gate. A
        hypothesis, not an order. None is valid.
      </p>
      {!pick && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>
          No close judgment yet. The scanner runs at 15:30 ET on trading days.
        </p>
      )}
      {pick && !ticker && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>{pick.why}</p>
      )}
      {pick && ticker && (
        <article>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'baseline' }}>
            <strong style={{ ...type.heading }}>{ticker}</strong>
            {pick.companyName && (
              <span style={{ ...type.micro, color: colors.textMuted }}>{pick.companyName}</span>
            )}
            <span style={{ ...type.micro, color: colors.textMuted }}>{pick.day}</span>
          </div>
          <p style={{ ...type.body, color: colors.text, margin: '8px 0 0' }}>{pick.why}</p>
        </article>
      )}
    </section>
  );
}

function SellSignalsSection({
  signals, threshold, colors,
}: {
  signals: SellSignal[];
  threshold: number;
  colors: ThemeColors;
}) {
  return (
    <section>
      <SectionTitle colors={colors}>Sell signals</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 8px' }}>
        Open holdings only. RSI-14 ≥ {threshold}, or two of: stochastic %K ≥ 80, 8% above the 20-day average, upper Bollinger band, within 2% of the 52-week high. A signal, not an order — nothing here sells.
      </p>
      {signals.length === 0 ? (
        <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>
          No open holding is showing overbought signs.
        </p>
      ) : (
        <ul style={{ margin: 0, paddingLeft: 18 }}>
          {signals.map((a) => (
            <li key={a.symbol} style={{ ...type.body, color: colors.danger, marginBottom: 8 }}>
              <div>{a.summary}</div>
              {a.signs.length > 0 && (
                <div style={{ ...type.micro, color: colors.textMuted }}>{a.signs.join(' · ')}</div>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function PicksSection({
  board, colors, busy, mutate, onPrefill,
}: {
  board: FinanceBoard;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  onPrefill: (draft: TradeDraft) => void;
}) {
  const picker = board.picker;
  return (
    <section>
      <SectionTitle colors={colors}>Validated picks</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 12px' }}>
        {picker.reachable
          ? `Scanner up at ${picker.baseUrl}${picker.scanInProgress ? ' — a scan is running' : picker.scanDate ? ` — last scan ${picker.scanDate}` : ''}${picker.results != null ? ` · ${picker.results} ranked` : ''}.`
          : `Scanner not reachable${picker.detail ? ` (${picker.detail})` : ''}.`}
        {' '}Yahoo quote + 1y daily closes. Loop gate (ICIR / half-life / out-of-sample) on the first eight. financialdatasets snapshot when the key is set. Ranker input is scan data only — holdings and bank balances never go in. Prefill lot fills the holdings form for a trade you already made; it does not place an order.
      </p>
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 12, alignItems: 'center' }}>
        <PickerControls picker={picker} colors={colors} busy={busy} mutate={mutate} />
        <button
          type="button"
          onClick={() => navigateToTool('world')}
          style={ghostBtn(colors)}
        >
          Open The Financier in World
        </button>
      </div>
      {board.picks.length === 0 ? (
        <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>No picks this cycle.</p>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {board.picks.map((p) => (
            <PickCard key={p.ticker} pick={p} colors={colors} onPrefill={onPrefill} />
          ))}
        </div>
      )}
    </section>
  );
}

function PickCard({
  pick, colors, onPrefill,
}: {
  pick: ValidatedPick;
  colors: ThemeColors;
  onPrefill: (draft: TradeDraft) => void;
}) {
  const loop = pick.loop;
  const yahoo = pick.quote?.price ?? null;
  const mark = yahoo ?? pick.pickerPrice ?? null;
  return (
    <article style={{ border: `1px solid ${colors.border}`, padding: '12px 14px', borderRadius: 6 }}>
      <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'baseline' }}>
        <strong>{pick.ticker}</strong>
        {pick.companyName && <span style={{ ...type.micro, color: colors.textMuted }}>{pick.companyName}</span>}
        {pick.tier && <span style={{ ...type.micro, color: colors.textMuted }}>{pick.tier}</span>}
        {pick.rank != null && <span style={{ ...type.micro, color: colors.textMuted }}>#{pick.rank}</span>}
        {loop && (
          <span style={{ ...type.micro, color: loop.passed ? colors.cyan : colors.danger, fontWeight: 600 }}>
            {loop.passed ? 'loop pass' : 'loop kill'}
          </span>
        )}
        {pick.priceMismatch && (
          <span style={{ ...type.micro, color: colors.danger }}>price mismatch (&gt;2%)</span>
        )}
        <button
          type="button"
          onClick={() => onPrefill({
            ticker: pick.ticker,
            company: pick.companyName ?? pick.ticker,
            date: todayIso(),
            price: mark != null ? String(mark) : '',
            shares: '',
            notes: pick.reason ? `Picker: ${pick.reason}` : '',
            exitDate: '',
            exitPrice: '',
            editingId: null,
            source: null,
          })}
          style={{ ...ghostBtn(colors), marginLeft: 'auto' }}
        >
          Prefill lot
        </button>
      </div>
      <div style={{ ...type.micro, color: colors.textMuted, marginTop: 6, display: 'flex', gap: 14, flexWrap: 'wrap' }}>
        <span>Scanner {fmtPrice(pick.pickerPrice)}</span>
        <span>Yahoo {pick.quoteError ? pick.quoteError : fmtPrice(yahoo, pick.quote?.currency)}</span>
        <span>RSI {pick.pickerRsi != null ? pick.pickerRsi.toFixed(1) : '—'}</span>
        {pick.score != null && <span>Score {pick.score.toFixed(1)}</span>}
        {pick.confidence != null && <span>Confidence {fmtNum(pick.confidence)}</span>}
        {pick.buyWindow && <span>Buy window {pick.buyWindow}</span>}
      </div>
      {pick.reason && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: '6px 0 0' }}>{pick.reason}</p>
      )}
      {loop && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: '6px 0 0' }}>
          ICIR {fmtNum(loop.icir)} · half-life {loop.halfLifeDays != null ? `${loop.halfLifeDays.toFixed(1)}d` : '—'} · OOS {fmtNum(loop.oosIcir)}
          {loop.kills.length > 0 ? ` · ${loop.kills.join('; ')}` : ''}
        </p>
      )}
      {pick.fundamentals.summary && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: '6px 0 0', whiteSpace: 'pre-wrap' }}>
          {pick.fundamentals.summary}
        </p>
      )}
      {!pick.fundamentals.summary && pick.fundamentals.error && (
        <p style={{ ...type.micro, color: colors.textMuted, margin: '6px 0 0' }}>{pick.fundamentals.error}</p>
      )}
    </article>
  );
}

function fmtNum(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '—';
  return n.toFixed(2);
}

function HouseholdSection({
  household, colors, busy, mutate, setError,
}: {
  household: HouseholdView;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  setError: (s: string | null) => void;
}) {
  const fileRef = useRef<HTMLInputElement>(null);
  const [hint, setHint] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);

  const onFiles = (files: FileList | File[]) => {
    const file = Array.from(files)[0];
    if (!file) return;
    setHint(`Reading ${file.name}…`);
    void mutate(async () => {
      try {
        const res = await uploadFinanceStatement(file);
        setHint(
          `Imported ${res.inserted} of ${res.parsed} lines from ${res.sourceFile}${res.ocrUsed ? ' (OCR)' : ''}.`,
        );
        setError(null);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setHint(msg);
        throw e;
      }
    });
  };

  return (
    <section>
      <SectionTitle colors={colors}>Household spend</SectionTitle>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 12px' }}>
        CSV/OFX/QFX first. PDF or screenshot uses OCR. Same-period CSV wins. Trailing run-rate, not a model.
      </p>
      <div
        onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragOver(false);
          if (e.dataTransfer.files.length) onFiles(e.dataTransfer.files);
        }}
        style={{
          border: `1px dashed ${dragOver ? colors.cyan : colors.border}`,
          borderRadius: 6,
          padding: '16px 14px',
          marginBottom: 14,
        }}
      >
        <p style={{ ...type.body, margin: '0 0 8px' }}>
          Drop a bank or card export here (CSV, OFX, QFX, or a PDF/screenshot).
        </p>
        <button type="button" disabled={busy} style={ghostBtn(colors)} onClick={() => fileRef.current?.click()}>
          Choose file
        </button>
        <input
          ref={fileRef}
          type="file"
          hidden
          accept=".csv,.ofx,.qfx,.pdf,.png,.jpg,.jpeg,.webp,.txt"
          onChange={(e) => {
            if (e.target.files?.length) onFiles(e.target.files);
            e.target.value = '';
          }}
        />
        {hint && <p style={{ ...type.micro, color: colors.textMuted, margin: '8px 0 0' }}>{hint}</p>}
      </div>
      <div style={{ display: 'flex', gap: 24, flexWrap: 'wrap', marginBottom: 8 }}>
        <Stat colors={colors} label="30-day run-rate" value={fmtPrice(household.forecast.runRate30d)} />
        <Stat colors={colors} label="90-day run-rate" value={fmtPrice(household.forecast.runRate90d)} />
        <Stat colors={colors} label="Spent (window)" value={fmtPrice(household.forecast.spend90d)} />
        <Stat colors={colors} label="Days in window" value={String(household.forecast.daysUsed)} />
      </div>
      <p style={{ ...type.micro, color: colors.textMuted, margin: '0 0 12px' }}>{household.forecast.method}</p>
      {household.forecast.byCategory.length > 0 && (
        <ul style={{ margin: '0 0 12px', paddingLeft: 18 }}>
          {household.forecast.byCategory.map((c) => (
            <li key={c.category} style={type.micro}>
              {c.category} · {fmtPrice(c.amount)} / window
            </li>
          ))}
        </ul>
      )}
      {household.recent.length === 0 ? (
        <p style={{ ...type.micro, color: colors.textMuted, margin: 0 }}>No imported transactions yet.</p>
      ) : (
        <table style={tableStyle(colors)}>
          <thead>
            <tr>
              <th style={th(colors)}>Date</th>
              <th style={th(colors)}>Payee</th>
              <th style={th(colors)}>Amount</th>
              <th style={th(colors)}>Category</th>
              <th style={th(colors)}>Source</th>
            </tr>
          </thead>
          <tbody>
            {household.recent.map((t) => (
              <tr key={t.id}>
                <td style={td(colors)}>{t.date}</td>
                <td style={td(colors)}>{t.payee}</td>
                <td style={{ ...td(colors), color: t.amount < 0 ? colors.danger : colors.text }}>{fmtSigned(t.amount)}</td>
                <td style={td(colors)}>
                  <select
                    value={t.category}
                    disabled={busy}
                    onChange={(e) => void mutate(() =>
                      apiFetch(`/api/finance/transactions/${encodeURIComponent(t.id)}`, {
                        method: 'PATCH',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ category: e.target.value }),
                      }),
                    )}
                    style={inputStyle(colors)}
                  >
                    {CATEGORIES.map((c) => (
                      <option key={c} value={c}>{c}</option>
                    ))}
                  </select>
                </td>
                <td style={{ ...td(colors), ...type.micro, color: colors.textMuted }}>{t.sourceFile || '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
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

function SectionTitle({ children, colors }: { children: string; colors: ThemeColors }) {
  return (
    <h2 style={{ ...type.micro, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textMuted, margin: '0 0 10px' }}>
      {children}
    </h2>
  );
}

function tableStyle(_colors: ThemeColors): CSSProperties {
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
function ghostBtnOn(colors: ThemeColors): CSSProperties {
  return {
    ...ghostBtn(colors),
    borderColor: colors.cyan,
    color: colors.cyan,
  };
}
