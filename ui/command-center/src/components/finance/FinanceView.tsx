/**
 * Finance — money board.
 *
 * Cards match the rest of the app (Projects Panel: veil, hairline, uppercase
 * 11px title). Polybot and Picker stay off until the user turns them on.
 * Holdings, household, watchlist, and notes are the default surface.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, FormEvent, ReactNode } from 'react';
import { font, radius, tabularNums, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { api, apiFetch, uploadFinanceStatement } from '../../lib/api';
import { ViewHeader } from '../common/ViewHeader';
import { navigateToTool } from '../../lib/store';
import { AGENT_TRIM } from '../world/shared/palette';
import { PolybotKeys } from './PolybotKeys';
import { FundamentalsKey } from './FundamentalsKey';
import { sparklinePolyline, sparklineZeroY } from '../grow/growthTrend';
import {
  PICKER_DISCLAIMER,
  PICKER_ENABLED_KEY,
  PICKER_UNIVERSE_KEY,
  PICKS_PREVIEW,
  POLYBOT_DISCLAIMER,
  POLYBOT_ENABLED_KEY,
  parseUniverse,
  appendUniverse,
  pickIsApproved,
  sortPicks,
} from './financeLabs';

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
  running?: boolean;
  pid?: number | null;
  paused: boolean;
  credentialsReady?: boolean;
  credentialsPath?: string | null;
  scanRequested?: boolean;
  quietHours?: boolean;
  currentBalance?: number | null;
  realizedPnl?: number | null;
  openExposure?: number | null;
  tradeCount?: number | null;
  lastUpdated?: string | null;
  asOf?: string | null;
  staleDays?: number | null;
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
  trend?: number[];
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
  polybotEnabled?: boolean;
  holdings: HoldingsView;
  watchlist: WatchlistRow[];
  notes: FinanceNote[];
  positions: Position[];
  picker: PickerStatus;
  pickerEnabled?: boolean;
  pickerUniverse?: string[];
  pickerUniverseCount?: number | null;
  fundamentalsConfigured?: boolean;
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

function fmtMoney(n: number | null | undefined, currency = 'USD'): string {
  if (n == null || Number.isNaN(n)) return '—';
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency: currency.length === 3 ? currency : 'USD',
      maximumFractionDigits: 2,
    }).format(n);
  } catch {
    return n.toFixed(2);
  }
}

function fmtPct(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '';
  const sign = n > 0 ? '+' : '';
  return `${sign}${n.toFixed(2)}%`;
}

function fmtSigned(n: number | null | undefined, currency = 'USD'): string {
  if (n == null || Number.isNaN(n)) return '—';
  const abs = fmtMoney(Math.abs(n), currency);
  if (n < 0) return `−${abs}`;
  if (n > 0) return `+${abs}`;
  return abs;
}

function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

function toneFor(n: number | null | undefined, colors: ThemeColors): string {
  if (n == null || Number.isNaN(n) || n === 0) return colors.text;
  return n < 0 ? colors.danger : colors.success;
}

export function FinanceView() {
  const { colors } = useTheme();
  const [board, setBoard] = useState<FinanceBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<TradeDraft>(emptyDraft);
  const [optIn, setOptIn] = useState({ polybot: false, picker: false });

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
    let live = true;
    void Promise.all([
      api.readConfig(POLYBOT_ENABLED_KEY).then((v) => v === true).catch(() => false),
      api.readConfig(PICKER_ENABLED_KEY).then((v) => v === true).catch(() => false),
    ]).then(([polybot, picker]) => {
      if (live) setOptIn({ polybot, picker });
    });
    return () => { live = false; };
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

  const view: FinanceBoard | null = board && {
    ...board,
    polybotEnabled: board.polybotEnabled ?? optIn.polybot,
    pickerEnabled: board.pickerEnabled ?? optIn.picker,
    pickerUniverse: board.pickerUniverse ?? [],
    pickerUniverseCount: board.pickerUniverseCount ?? null,
    fundamentalsConfigured: board.fundamentalsConfigured ?? false,
  };

  const setLab = (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) =>
    mutate(async () => {
      await api.upsertConfig(key, on);
      setOptIn((s) => (
        key === POLYBOT_ENABLED_KEY ? { ...s, polybot: on } : { ...s, picker: on }
      ));
    });

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', fontFamily: font.body, color: colors.text }}>
      <ViewHeader
        title="Finance"
        subtitle="Holdings, household, and research. Optional desks stay off until you turn them on."
        actions={
          <button type="button" onClick={() => { void load(); }} style={ghostBtn(colors)}>
            Refresh
          </button>
        }
      />
      <div style={{ flex: 1, overflow: 'auto', padding: '20px 24px 48px' }}>
        {error && (
          <div style={{
            ...type.small,
            color: colors.danger,
            background: warnFill(colors.danger),
            border: `1px solid ${warnFill(colors.danger, 0.35)}`,
            borderRadius: radius.md,
            padding: '10px 14px',
            marginBottom: 16,
          }}
          >
            {error}
          </div>
        )}
        {!board && !error && (
          <div style={{ ...type.small, color: colors.textMuted }}>Loading…</div>
        )}
        {view && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 1120 }}>
            <SummaryStrip
              board={view}
              colors={colors}
              busy={busy}
              mutate={mutate}
              setLab={setLab}
            />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <HoldingsSection
                holdings={view.holdings}
                rsiThreshold={view.rsiThreshold}
                picker={view.picker}
                colors={colors}
                busy={busy}
                mutate={mutate}
                draft={draft}
                setDraft={setDraft}
                onRecorded={(hint) => { if (hint) setError(hint); }}
              />
              {view.pickerEnabled && (
                <PicksSection
                  board={view}
                  colors={colors}
                  onPrefill={(next) => {
                    setDraft(next);
                    requestAnimationFrame(() => {
                      document.getElementById('finance-holdings-form')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
                    });
                  }}
                />
              )}
            </div>
            <HouseholdSection
              household={view.household}
              colors={colors}
              busy={busy}
              mutate={mutate}
              setError={setError}
            />
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))', gap: 16 }}>
              <WatchlistSection board={view} colors={colors} busy={busy} mutate={mutate} />
              <NotesSection board={view} colors={colors} busy={busy} mutate={mutate} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function SummaryStrip({
  board, colors, busy, mutate, setLab,
}: {
  board: FinanceBoard;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<void>;
}) {
  const p = board.polybot;
  const asOf = p.asOf || p.lastUpdated;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))', gap: 12, alignItems: 'start' }}>
        {board.polybotEnabled && (
          <Card colors={colors} warn={p.stale} testId="finance-polybot-card">
            <Eyebrow colors={colors}>Polybot</Eyebrow>
            <Hero
              colors={colors}
              value={fmtMoney(p.currentBalance)}
              tone={p.stale ? colors.warning : colors.text}
            />
            <div style={{ ...type.caption, color: colors.textMuted, marginTop: 4 }}>
              {p.stale
                ? `As of ${fmtWhen(asOf)}${p.staleDays != null ? ` · ${p.staleDays}d stale` : ''}`
                : p.paused
                  ? 'Paused · live file'
                  : `Live file · ${fmtWhen(asOf)}`}
            </div>
            <div style={{ display: 'flex', gap: 14, marginTop: 10, flexWrap: 'wrap' }}>
              <Mini colors={colors} label="Realized" value={fmtSigned(p.realizedPnl)} tone={toneFor(p.realizedPnl, colors)} />
              <Mini colors={colors} label="Open" value={fmtMoney(p.openExposure)} />
              <Mini colors={colors} label="Trades" value={p.tradeCount != null ? String(p.tradeCount) : '—'} />
            </div>
            <div style={{ ...type.caption, color: colors.textMuted, marginTop: 8 }}>
              {p.running ? `Running${p.pid != null ? ` · pid ${p.pid}` : ''}` : 'Process down'}
              {p.paused ? ' · paused' : ''}
              {p.credentialsReady ? ' · keys in keychain' : ' · keys missing'}
            </div>
            <div style={{ marginTop: 10 }}>
              <PolybotControls polybot={p} colors={colors} busy={busy} mutate={mutate} setLab={setLab} />
            </div>
          </Card>
        )}

        <Card colors={colors} testId="finance-holdings-card">
          <Eyebrow colors={colors}>Holdings</Eyebrow>
          <Hero colors={colors} value={fmtSigned(board.holdings.netPnl)} tone={toneFor(board.holdings.netPnl, colors)} />
          <div style={{ ...type.caption, color: colors.textMuted, marginTop: 4 }}>
            Net P&amp;L · {board.holdings.openCount} open
            {' · '}
            {board.holdings.source === 'picker' ? 'Picker journal' : 'local ledger'}
          </div>
          <div style={{ display: 'flex', gap: 14, marginTop: 10, flexWrap: 'wrap' }}>
            <Mini colors={colors} label="Unrealized" value={fmtSigned(board.holdings.netUnrealized)} tone={toneFor(board.holdings.netUnrealized, colors)} />
            <Mini colors={colors} label="Realized" value={fmtSigned(board.holdings.netRealized)} tone={toneFor(board.holdings.netRealized, colors)} />
          </div>
          <HoldingsSparkline values={board.holdings.trend ?? []} colors={colors} />
        </Card>

        {board.pickerEnabled && (
          <Card colors={colors} warn={!board.picker.reachable && (board.pickerUniverse?.length ?? 0) === 0} testId="finance-picker-card">
            <Eyebrow colors={colors}>Picker</Eyebrow>
            <Hero
              colors={colors}
              value={board.picker.reachable ? (board.picker.scanInProgress ? 'Scanning' : 'Up') : ((board.pickerUniverse?.length ?? 0) ? 'Universe' : 'Idle')}
              tone={board.picker.reachable ? colors.success : colors.text}
            />
            <div style={{ ...type.caption, color: colors.textMuted, marginTop: 4 }}>
              {(board.pickerUniverseCount ?? 0) > 0
                ? `Picker universe · ${board.pickerUniverseCount?.toLocaleString()} names`
                : board.picker.reachable
                  ? `${board.picker.results != null ? `${board.picker.results} ranked` : 'ready'}${board.picker.scanDate ? ` · ${board.picker.scanDate}` : ''}`
                  : 'Connected when the scanner is up'}
              {(board.pickerUniverse?.length ?? 0) > 0
                ? ` · ${board.pickerUniverse?.length} extra${board.pickerUniverse?.length === 1 ? '' : 's'} you added`
                : ''}
            </div>
            <div style={{ marginTop: 10 }}>
              <PickerControls
                picker={board.picker}
                universe={board.pickerUniverse ?? []}
                colors={colors}
                busy={busy}
                mutate={mutate}
                setLab={setLab}
              />
            </div>
          </Card>
        )}
      </div>
      {(!board.polybotEnabled || !board.pickerEnabled) && (
        <LabsRow
          polybotOn={Boolean(board.polybotEnabled)}
          pickerOn={Boolean(board.pickerEnabled)}
          colors={colors}
          busy={busy}
          setLab={setLab}
        />
      )}
      <FundamentalsKey compact onChanged={() => void mutate(async () => undefined)} />
    </div>
  );
}

function LabsRow({
  polybotOn, pickerOn, colors, busy, setLab,
}: {
  polybotOn: boolean;
  pickerOn: boolean;
  colors: ThemeColors;
  busy: boolean;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<void>;
}) {
  const [dialog, setDialog] = useState<'polybot' | 'picker' | null>(null);
  return (
    <>
      <div
        data-testid="finance-labs-row"
        style={{
          display: 'flex',
          gap: 8,
          flexWrap: 'wrap',
          alignItems: 'center',
        }}
      >
        {!polybotOn && (
          <button
            type="button"
            data-testid="finance-enable-polybot"
            disabled={busy}
            onClick={() => setDialog('polybot')}
            style={ghostBtn(colors)}
          >
            Turn on Polybot
          </button>
        )}
        {!pickerOn && (
          <button
            type="button"
            data-testid="finance-enable-picker"
            disabled={busy}
            onClick={() => setDialog('picker')}
            style={ghostBtn(colors)}
          >
            Turn on Picker
          </button>
        )}
        <span style={{ ...type.caption, color: colors.textMuted }}>
          Optional desks. Off until you opt in.
        </span>
      </div>
      {dialog === 'polybot' && (
        <DisclaimerDialog
          title="Turn on Polybot"
          body={POLYBOT_DISCLAIMER}
          confirmLabel="I understand this can lose real money"
          colors={colors}
          busy={busy}
          onCancel={() => setDialog(null)}
          onConfirm={() => {
            void setLab(POLYBOT_ENABLED_KEY, true).then(() => setDialog(null));
          }}
        />
      )}
      {dialog === 'picker' && (
        <DisclaimerDialog
          title="Turn on Picker"
          body={PICKER_DISCLAIMER}
          confirmLabel="Enable Picker"
          colors={colors}
          busy={busy}
          requireCheck={false}
          onCancel={() => setDialog(null)}
          onConfirm={() => {
            void setLab(PICKER_ENABLED_KEY, true).then(() => setDialog(null));
          }}
        />
      )}
    </>
  );
}

function DisclaimerDialog({
  title, body, confirmLabel, colors, busy, onCancel, onConfirm, requireCheck = true,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  colors: ThemeColors;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  requireCheck?: boolean;
}) {
  const [checked, setChecked] = useState(!requireCheck);
  return (
    <div
      data-testid="finance-disclaimer"
      style={{
        border: `1px solid ${colors.border}`,
        borderRadius: 10,
        padding: '12px 14px',
        background: colors.bgDeeper,
        maxWidth: 560,
      }}
    >
      <div style={{ ...type.label, color: colors.textMuted, marginBottom: 8 }}>{title}</div>
      <p style={{ ...type.small, color: colors.text, margin: 0, lineHeight: 1.5 }}>{body}</p>
      {requireCheck && (
        <label style={{ display: 'flex', gap: 8, alignItems: 'flex-start', marginTop: 12, ...type.caption, color: colors.text }}>
          <input
            type="checkbox"
            data-testid="finance-disclaimer-check"
            checked={checked}
            onChange={(e) => setChecked(e.target.checked)}
            style={{ marginTop: 2 }}
          />
          <span>I have read this. The bot can place real orders.</span>
        </label>
      )}
      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <button
          type="button"
          disabled={busy || !checked}
          onClick={onConfirm}
          style={primaryBtn(colors)}
        >
          {confirmLabel}
        </button>
        <button type="button" disabled={busy} onClick={onCancel} style={ghostBtn(colors)}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function PolybotControls({
  polybot, colors, busy, mutate, setLab,
}: {
  polybot: PolybotStatus;
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<void>;
}) {
  const [showKeys, setShowKeys] = useState(false);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
        <button
          type="button"
          disabled={busy || (Boolean(polybot.running) && !polybot.paused)}
          onClick={() => void mutate(() =>
            apiFetch('/api/finance/polybot/start', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
          style={polybot.running && !polybot.paused ? ghostBtn(colors) : primaryBtn(colors)}
        >
          {polybot.running && !polybot.paused ? 'Running' : polybot.paused ? 'Resume' : 'Start'}
        </button>
        <button
          type="button"
          disabled={busy || !polybot.running || polybot.paused}
          onClick={() => void mutate(() =>
            apiFetch('/api/finance/polybot/pause', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
          style={ghostBtn(colors)}
        >
          Pause
        </button>
        <button
          type="button"
          disabled={busy || polybot.paused || polybot.scanRequested}
          onClick={() => void mutate(() =>
            apiFetch('/api/finance/polybot/scan', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
          style={ghostBtn(colors)}
        >
          {polybot.scanRequested ? 'Scan queued…' : 'Scan now'}
        </button>
        <button
          type="button"
          onClick={() => setShowKeys((v) => !v)}
          style={ghostBtn(colors)}
        >
          {showKeys ? 'Hide keys' : 'Keys'}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => void setLab(POLYBOT_ENABLED_KEY, false)}
          style={ghostBtn(colors)}
        >
          Turn off
        </button>
      </div>
      {showKeys && (
        <PolybotKeys compact onChanged={() => void mutate(async () => undefined)} />
      )}
    </div>
  );
}

function PickerControls({
  picker, universe, colors, busy, mutate, setLab,
}: {
  picker: PickerStatus;
  universe: string[];
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<void>;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<void>;
}) {
  const [showAdd, setShowAdd] = useState(false);
  const [draft, setDraft] = useState('');

  const saveExtras = (next: string[]) =>
    mutate(() => api.upsertConfig(PICKER_UNIVERSE_KEY, next.join('\n')));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
        <button
          type="button"
          disabled={busy || picker.reachable}
          onClick={() => void mutate(() =>
            apiFetch('/api/finance/picker/start', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
          style={ghostBtn(colors)}
        >
          {picker.reachable ? 'Scanner up' : 'Start scanner'}
        </button>
        <button
          type="button"
          disabled={busy || picker.scanInProgress}
          onClick={() => void mutate(() =>
            apiFetch('/api/finance/picker/scan', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
          style={ghostBtn(colors)}
        >
          {picker.scanInProgress ? 'Scan running…' : 'Run scan'}
        </button>
        <button
          type="button"
          onClick={() => setShowAdd((v) => !v)}
          style={ghostBtn(colors)}
        >
          {showAdd ? 'Hide add' : 'Add ticker'}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => void setLab(PICKER_ENABLED_KEY, false)}
          style={ghostBtn(colors)}
        >
          Turn off
        </button>
      </div>
      {universe.length > 0 && (
        <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }} data-testid="picker-extras">
          {universe.map((t) => (
            <span
              key={t}
              style={{
                ...type.micro,
                display: 'inline-flex',
                alignItems: 'center',
                gap: 4,
                border: `1px solid ${colors.border}`,
                borderRadius: radius.sm,
                padding: '2px 6px',
                fontFamily: font.mono,
              }}
            >
              {t}
              <button
                type="button"
                disabled={busy}
                aria-label={`Remove ${t}`}
                onClick={() => void saveExtras(universe.filter((x) => x !== t))}
                style={{ ...ghostBtn(colors), padding: '0 4px', border: 'none' }}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
      {showAdd && (
        <form
          data-testid="picker-universe"
          onSubmit={(e) => {
            e.preventDefault();
            const next = appendUniverse(universe, draft);
            void saveExtras(next).then(() => setDraft(''));
          }}
          style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}
        >
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="AAPL, SHOP.TO — added to your universe"
            style={{ ...inputStyle(colors), flex: 1, minWidth: 140 }}
          />
          <button type="submit" disabled={busy || parseUniverse(draft).length === 0} style={primaryBtn(colors)}>
            Add
          </button>
        </form>
      )}
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
  const [filter, setFilter] = useState<'open' | 'all'>('open');
  const [showForm, setShowForm] = useState(false);
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
      setShowForm(false);
      onRecorded(
        res.pickerError
          ? `Saved on this tab. Scanner history was not updated (${res.pickerError}).`
          : null,
      );
    });
  };

  const onEdit = (row: HoldingRow) => {
    setDraft(draftFromRow(row));
    setShowForm(true);
    requestAnimationFrame(() => {
      document.getElementById('finance-holdings-form')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    });
  };

  return (
    <Card colors={colors}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 12, marginBottom: 12 }}>
        <SectionTitle colors={colors}>Lots</SectionTitle>
        <button
          type="button"
          onClick={() => { setShowForm((s) => !s); if (editing) setDraft(emptyDraft()); }}
          style={ghostBtn(colors)}
        >
          {showForm || editing ? 'Hide form' : 'Record a trade'}
        </button>
      </div>
      <p style={{ ...type.caption, color: colors.textMuted, margin: '0 0 12px' }}>
        Journal of trades already made. Does not buy or sell
        {!picker.reachable ? ' — scanner is down, lots stay on this tab.' : '.'}
      </p>
      {(showForm || editing) && (
        <form
          id="finance-holdings-form"
          onSubmit={onSubmit}
          style={{
            border: `1px solid ${colors.border}`,
            borderRadius: radius.sm,
            padding: 12,
            marginBottom: 14,
            display: 'flex',
            flexDirection: 'column',
            gap: 10,
            background: colors.bgDeeper,
          }}
        >
          <div style={{ ...type.label, color: colors.textMuted }}>
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
            <Field colors={colors} label="Exit date">
              <input value={draft.exitDate} onChange={(e) => setDraft({ ...draft, exitDate: e.target.value })} placeholder="YYYY-MM-DD" style={inputStyle(colors)} />
            </Field>
            <Field colors={colors} label="Exit price">
              <input value={draft.exitPrice} onChange={(e) => setDraft({ ...draft, exitPrice: e.target.value })} placeholder="0.00" style={inputStyle(colors)} />
            </Field>
            <Field colors={colors} label="Notes" wide>
              <input value={draft.notes} onChange={(e) => setDraft({ ...draft, notes: e.target.value })} placeholder="Why you took it" style={{ ...inputStyle(colors), minWidth: 220 }} />
            </Field>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button type="submit" disabled={busy} style={primaryBtn(colors)}>
              {editing ? 'Save trade' : 'Record trade'}
            </button>
            {editing && (
              <button type="button" disabled={busy} onClick={() => { setDraft(emptyDraft()); setShowForm(false); }} style={ghostBtn(colors)}>
                Cancel
              </button>
            )}
          </div>
        </form>
      )}
      {holdings.rows.length === 0 ? (
        <p style={{ ...type.small, color: colors.textMuted, margin: 0 }}>
          No lots yet. Record a trade you already made.
        </p>
      ) : (
        <>
          <div style={{ display: 'flex', gap: 6, marginBottom: 10 }}>
            <button type="button" onClick={() => setFilter('open')} style={filter === 'open' ? ghostBtnOn(colors) : ghostBtn(colors)}>
              Open ({holdings.openCount})
            </button>
            <button type="button" onClick={() => setFilter('all')} style={filter === 'all' ? ghostBtnOn(colors) : ghostBtn(colors)}>
              All ({holdings.rows.length})
            </button>
          </div>
          <div style={{ overflowX: 'auto', minWidth: 0 }}>
          <table style={{ ...tableStyle(), tableLayout: 'fixed' }}>
            <thead>
              <tr>
                <th style={th(colors)}>Symbol</th>
                <th style={{ ...th(colors), textAlign: 'right' }}>Shares</th>
                <th style={{ ...th(colors), textAlign: 'right' }}>Mark</th>
                <th style={{ ...th(colors), textAlign: 'right' }}>P&amp;L</th>
                <th style={{ ...th(colors), textAlign: 'right' }}>RSI</th>
                <th style={th(colors)} />
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => {
                const closed = Boolean(p.exitDate);
                const rsiHot = Boolean(p.sellSignal) || (!closed && p.rsi != null && p.rsi >= rsiThreshold);
                const pnl = closed ? p.realized : p.unrealized;
                return (
                  <tr key={p.id} style={closed ? { opacity: 0.55 } : undefined}>
                    <td style={td(colors)}>
                      <div style={{ fontWeight: 600 }}>{p.symbol}</div>
                      <div style={{ ...type.caption, color: colors.textMuted }}>
                        {closed ? `${p.entryDate} → ${p.exitDate}` : p.entryDate}
                      </div>
                    </td>
                    <td style={{ ...td(colors), textAlign: 'right', ...tabularNums }}>{p.shares}</td>
                    <td style={{ ...td(colors), textAlign: 'right', ...tabularNums }}>
                      {p.quoteError ? (
                        <span style={{ color: colors.textMuted }}>{p.quoteError}</span>
                      ) : closed ? (
                        fmtMoney(p.exitPrice)
                      ) : (
                        fmtMoney(p.last, p.quote?.currency ?? undefined)
                      )}
                    </td>
                    <td style={{ ...td(colors), textAlign: 'right', ...tabularNums, color: toneFor(pnl, colors), fontWeight: 600 }}>
                      {fmtSigned(pnl)}
                      {!closed && p.unrealizedPct != null ? (
                        <div style={{ ...type.caption, color: colors.textMuted, fontWeight: 400 }}>{fmtPct(p.unrealizedPct)}</div>
                      ) : null}
                    </td>
                    <td style={{ ...td(colors), textAlign: 'right', ...tabularNums, color: rsiHot ? colors.danger : colors.text }}>
                      {p.rsi != null ? p.rsi.toFixed(1) : '—'}
                    </td>
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
          </div>
        </>
      )}
    </Card>
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
  const [closing, setClosing] = useState(false);
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
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, alignItems: 'flex-end', minWidth: 0 }}>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', justifyContent: 'flex-end' }}>
        <button type="button" disabled={busy} onClick={onEdit} style={ghostBtn(colors)}>Edit</button>
        {!row.exitDate && (
          <button type="button" disabled={busy} onClick={() => setClosing((c) => !c)} style={ghostBtn(colors)}>
            Close
          </button>
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
      {closing && !row.exitDate && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void mutate(() =>
              apiFetch(closePath, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ exitDate, exitPrice: Number(exitPrice) }),
              }),
            );
          }}
          style={{ display: 'flex', gap: 6 }}
        >
          <input value={exitDate} onChange={(e) => setExitDate(e.target.value)} style={{ ...inputStyle(colors), minWidth: 110 }} required />
          <input value={exitPrice} onChange={(e) => setExitPrice(e.target.value)} style={{ ...inputStyle(colors), minWidth: 80 }} required />
          <button type="submit" disabled={busy} style={primaryBtn(colors)}>Mark closed</button>
        </form>
      )}
    </div>
  );
}

function PicksSection({
  board, colors, onPrefill,
}: {
  board: FinanceBoard;
  colors: ThemeColors;
  onPrefill: (draft: TradeDraft) => void;
}) {
  const approved = board.dailyPick?.ticker ?? null;
  const ranked = sortPicks(board.picks, approved);
  const [showAll, setShowAll] = useState(false);
  const visible = showAll ? ranked : ranked.slice(0, PICKS_PREVIEW);
  const hidden = ranked.length - visible.length;

  return (
    <Card colors={colors} testId="finance-picks-card">
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8, marginBottom: 8 }}>
        <SectionTitle colors={colors}>Picks</SectionTitle>
        <button type="button" onClick={() => navigateToTool('world')} style={ghostBtn(colors)}>
          Financier
        </button>
      </div>
      <p style={{ ...type.caption, color: colors.textMuted, margin: '0 0 10px' }}>
        Your universe, Yahoo + loop gate. Gold means the Financier approved it.
      </p>
      {ranked.length === 0 ? (
        <p style={{ ...type.small, color: colors.textMuted, margin: 0 }}>
          {(board.pickerUniverse?.length ?? 0) === 0 && !(board.pickerUniverseCount)
            ? 'Add tickers, or run a scan on the Picker universe.'
            : 'No picks this cycle.'}
        </p>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          {visible.map((p) => (
            <PickRow
              key={p.ticker}
              pick={p}
              approved={pickIsApproved(p.ticker, approved)}
              colors={colors}
              onPrefill={onPrefill}
            />
          ))}
          {hidden > 0 && (
            <button
              type="button"
              onClick={() => setShowAll(true)}
              style={{ ...ghostBtn(colors), alignSelf: 'flex-start', marginTop: 8 }}
            >
              Show {hidden} more
            </button>
          )}
        </div>
      )}
    </Card>
  );
}

function PickRow({
  pick, approved, colors, onPrefill,
}: {
  pick: ValidatedPick;
  approved: boolean;
  colors: ThemeColors;
  onPrefill: (draft: TradeDraft) => void;
}) {
  const [open, setOpen] = useState(approved);
  const loop = pick.loop;
  const yahoo = pick.quote?.price ?? null;
  const mark = yahoo ?? pick.pickerPrice ?? null;
  return (
    <article
      data-testid="pick-row"
      data-approved={approved ? 'true' : 'false'}
      style={{
        borderTop: `1px solid ${colors.border}`,
        padding: '8px 0',
      }}
    >
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'baseline' }}>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          style={{
            background: 'none',
            border: 'none',
            padding: 0,
            cursor: 'pointer',
            fontFamily: font.body,
            color: colors.text,
            display: 'flex',
            gap: 8,
            alignItems: 'baseline',
            flex: 1,
            minWidth: 0,
            textAlign: 'left',
          }}
        >
          <strong style={{ fontSize: 13, fontWeight: 600 }}>{pick.ticker}</strong>
          {pick.companyName && (
            <span style={{ ...type.caption, color: colors.textMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {pick.companyName}
            </span>
          )}
        </button>
        {approved && (
          <span
            data-testid="pick-financier-badge"
            style={{
              ...type.micro,
              color: '#3d2e0a',
              background: AGENT_TRIM.financier,
              fontWeight: 700,
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
              padding: '2px 7px',
              borderRadius: 999,
            }}
          >
            Agent approved
          </span>
        )}
        {loop && (
          <span style={{ ...type.micro, color: loop.passed ? colors.success : colors.danger, fontWeight: 600 }}>
            {loop.passed ? 'loop pass' : 'loop kill'}
          </span>
        )}
        <span style={{ ...type.caption, color: colors.textMuted, ...tabularNums }}>
          {fmtMoney(yahoo ?? pick.pickerPrice, pick.quote?.currency ?? undefined)}
        </span>
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
          style={ghostBtn(colors)}
        >
          Prefill
        </button>
      </div>
      {open && (
        <div style={{ ...type.caption, color: colors.textMuted, marginTop: 6, display: 'flex', flexDirection: 'column', gap: 4 }}>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', ...tabularNums }}>
            <span>Scan {fmtMoney(pick.pickerPrice)}</span>
            <span>Yahoo {pick.quoteError ? pick.quoteError : fmtMoney(yahoo, pick.quote?.currency ?? undefined)}</span>
            <span>RSI {pick.pickerRsi != null ? pick.pickerRsi.toFixed(1) : '—'}</span>
            {pick.score != null && <span>Score {pick.score.toFixed(1)}</span>}
          </div>
          {pick.reason && <p style={{ margin: 0 }}>{pick.reason}</p>}
          {loop && (
            <p style={{ margin: 0 }}>
              ICIR {fmtNum(loop.icir)} · half-life {loop.halfLifeDays != null ? `${loop.halfLifeDays.toFixed(1)}d` : '—'}
              {loop.kills.length > 0 ? ` · ${loop.kills.join('; ')}` : ''}
            </p>
          )}
        </div>
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
        setHint(`Imported ${res.inserted} of ${res.parsed} lines from ${res.sourceFile}${res.ocrUsed ? ' (OCR)' : ''}.`);
        setError(null);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setHint(msg);
        throw e;
      }
    });
  };

  return (
    <Card colors={colors}>
      <SectionTitle colors={colors}>Household</SectionTitle>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))', gap: 12, margin: '12px 0 16px' }}>
        <Mini colors={colors} label="30-day run-rate" value={fmtMoney(household.forecast.runRate30d)} large />
        <Mini colors={colors} label="90-day run-rate" value={fmtMoney(household.forecast.runRate90d)} large />
        <Mini colors={colors} label="Spent (window)" value={fmtMoney(household.forecast.spend90d)} large />
        <Mini colors={colors} label="Days" value={String(household.forecast.daysUsed)} large />
      </div>
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
          borderRadius: radius.sm,
          padding: '12px 14px',
          marginBottom: 14,
        }}
      >
        <span style={{ ...type.small, color: colors.textMuted }}>
          Drop a CSV, OFX, QFX, or a PDF/screenshot
        </span>
        {' '}
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
        {hint && <p style={{ ...type.caption, color: colors.textMuted, margin: '8px 0 0' }}>{hint}</p>}
      </div>
      {household.forecast.byCategory.length > 0 && (
        <div style={{ display: 'flex', gap: 16, flexWrap: 'wrap', marginBottom: 12 }}>
          {household.forecast.byCategory.slice(0, 6).map((c) => (
            <Mini key={c.category} colors={colors} label={c.category} value={fmtMoney(c.amount)} />
          ))}
        </div>
      )}
      {household.recent.length === 0 ? (
        <p style={{ ...type.small, color: colors.textMuted, margin: 0 }}>No imported transactions yet.</p>
      ) : (
        <table style={tableStyle()}>
          <thead>
            <tr>
              <th style={th(colors)}>Date</th>
              <th style={th(colors)}>Payee</th>
              <th style={{ ...th(colors), textAlign: 'right' }}>Amount</th>
              <th style={th(colors)}>Category</th>
            </tr>
          </thead>
          <tbody>
            {household.recent.map((t) => (
              <tr key={t.id}>
                <td style={{ ...td(colors), ...tabularNums, color: colors.textMuted }}>{t.date}</td>
                <td style={td(colors)}>{t.payee}</td>
                <td style={{ ...td(colors), textAlign: 'right', ...tabularNums, color: toneFor(t.amount, colors) }}>
                  {fmtSigned(t.amount)}
                </td>
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
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
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
    <Card colors={colors}>
      <SectionTitle colors={colors}>Watchlist</SectionTitle>
      {board.watchlist.length === 0 && (
        <p style={{ ...type.small, color: colors.textMuted, margin: '0 0 12px' }}>
          Empty. Add a ticker.
        </p>
      )}
      {board.watchlist.length > 0 && (
        <table style={tableStyle()}>
          <thead>
            <tr>
              <th style={th(colors)}>Symbol</th>
              <th style={{ ...th(colors), textAlign: 'right' }}>Price</th>
              <th style={th(colors)} />
            </tr>
          </thead>
          <tbody>
            {board.watchlist.map((row) => {
              const q = row.quote;
              return (
                <tr key={row.id}>
                  <td style={td(colors)}>
                    <div style={{ fontWeight: 600 }}>{row.symbol}</div>
                    <div style={{ ...type.caption, color: colors.textMuted }}>{row.label || q?.name || ''}</div>
                  </td>
                  <td style={{ ...td(colors), textAlign: 'right', ...tabularNums }}>
                    {row.quoteError ? (
                      <span style={{ color: colors.textMuted }}>{row.quoteError}</span>
                    ) : (
                      <>
                        <div>{fmtMoney(q?.price, q?.currency ?? undefined)}</div>
                        <div style={{ ...type.caption, color: toneFor(q?.change ?? null, colors) }}>
                          {fmtPct(q?.changePercent)}
                        </div>
                      </>
                    )}
                  </td>
                  <td style={{ ...td(colors), textAlign: 'right' }}>
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
        <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Name" style={inputStyle(colors)} />
        <button type="submit" disabled={busy || !symbol.trim()} style={ghostBtn(colors)}>Add</button>
      </form>
    </Card>
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
    <Card colors={colors}>
      <SectionTitle colors={colors}>Notes</SectionTitle>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {board.notes.map((n) => (
          <div key={n.id} style={{ borderBottom: `1px solid ${colors.border}`, paddingBottom: 10 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12 }}>
              <div>
                <span style={{ fontWeight: 600 }}>{n.title}</span>
                {n.symbol && <span style={{ ...type.caption, color: colors.textMuted, marginLeft: 8 }}>{n.symbol}</span>}
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
            <p style={{ ...type.small, margin: '6px 0 0', whiteSpace: 'pre-wrap', color: colors.textMuted }}>{n.body}</p>
          </div>
        ))}
      </div>
      <form onSubmit={onAdd} style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 12 }}>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" style={inputStyle(colors)} required />
          <input value={symbol} onChange={(e) => setSymbol(e.target.value)} placeholder="Ticker" style={inputStyle(colors)} />
        </div>
        <textarea value={body} onChange={(e) => setBody(e.target.value)} placeholder="Observation — not a recommendation" style={{ ...inputStyle(colors), minHeight: 64 }} required />
        <button type="submit" disabled={busy} style={{ ...ghostBtn(colors), alignSelf: 'flex-start' }}>Add note</button>
      </form>
    </Card>
  );
}

function HoldingsSparkline({ values, colors }: { values: number[]; colors: ThemeColors }) {
  if (values.length < 2) return null;
  const W = 240;
  const H = 40;
  const poly = sparklinePolyline(values, W, H);
  const zeroY = sparklineZeroY(values, H);
  const n = values.length;
  let sumX = 0;
  let sumY = 0;
  let sumXY = 0;
  let sumXX = 0;
  values.forEach((y, i) => {
    sumX += i;
    sumY += y;
    sumXY += i * y;
    sumXX += i * i;
  });
  const denom = n * sumXX - sumX * sumX || 1;
  const slope = (n * sumXY - sumX * sumY) / denom;
  const intercept = (sumY - slope * sumX) / n;
  const fitted = values.map((_, i) => intercept + slope * i);
  const min = Math.min(0, ...values);
  const max = Math.max(0, ...values);
  const span = max - min || 1;
  const pad = 2;
  const innerW = Math.max(1, W - pad * 2);
  const innerH = Math.max(1, H - pad * 2);
  const yOf = (v: number) => pad + innerH - ((v - min) / span) * innerH;
  const x0 = pad;
  const x1 = pad + innerW;
  const trend = `${x0.toFixed(1)},${yOf(fitted[0]).toFixed(1)} ${x1.toFixed(1)},${yOf(fitted[n - 1]).toFixed(1)}`;

  return (
    <svg
      data-testid="holdings-sparkline"
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      width="100%"
      height={40}
      role="img"
      aria-label="Holdings net P&L trend"
      style={{ marginTop: 10, display: 'block' }}
    >
      {zeroY != null && (
        <line x1={0} x2={W} y1={zeroY} y2={zeroY} stroke={colors.border} strokeWidth={1} vectorEffect="non-scaling-stroke" />
      )}
      {poly && (
        <polyline points={poly} fill="none" stroke={colors.success} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke" />
      )}
      <polyline points={trend} fill="none" stroke={AGENT_TRIM.financier} strokeWidth={1.25} strokeDasharray="3 3" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

function Card({
  children, colors, warn, testId,
}: {
  children: ReactNode;
  colors: ThemeColors;
  warn?: boolean;
  testId?: string;
}) {
  const { theme } = useTheme();
  const veil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  return (
    <section
      data-testid={testId}
      style={{
        background: veil,
        border: `1px solid ${warn ? warnFill(colors.warning, 0.45) : colors.border}`,
        borderRadius: 10,
        padding: '14px 16px',
        overflow: 'hidden',
        minWidth: 0,
      }}
    >
      {children}
    </section>
  );
}

function Hero({ colors, value, tone }: { colors: ThemeColors; value: string; tone?: string }) {
  return (
    <div style={{
      fontFamily: font.display,
      fontSize: 19,
      fontWeight: 600,
      lineHeight: 1.15,
      letterSpacing: '-0.015em',
      ...tabularNums,
      color: tone ?? colors.text,
      marginTop: 2,
    }}
    >
      {value}
    </div>
  );
}

function Mini({
  colors, label, value, tone, large,
}: {
  colors: ThemeColors;
  label: string;
  value: string;
  tone?: string;
  large?: boolean;
}) {
  return (
    <div>
      <div style={{ ...type.label, color: colors.textMuted }}>{label}</div>
      <div style={{ ...(large ? type.heading : type.body), ...tabularNums, color: tone ?? colors.text, marginTop: 2 }}>
        {value}
      </div>
    </div>
  );
}

function Eyebrow({ children, colors }: { children: string; colors: ThemeColors }) {
  return <div style={{ ...type.label, color: colors.textMuted }}>{children}</div>;
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
    <label style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: wide ? 220 : 110, flex: wide ? 1 : undefined }}>
      <span style={{ ...type.label, color: colors.textMuted }}>{label}</span>
      {children}
    </label>
  );
}

function SectionTitle({ children, colors }: { children: string; colors: ThemeColors }) {
  return (
    <h2 style={{
      fontSize: 11,
      fontWeight: 600,
      color: colors.textMuted,
      textTransform: 'uppercase',
      letterSpacing: '0.06em',
      margin: 0,
    }}
    >
      {children}
    </h2>
  );
}

function warnFill(hex: string, a = 0.12): string {
  if (hex.startsWith('rgba')) return hex;
  const h = hex.replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r},${g},${b},${a})`;
}

function tableStyle(): CSSProperties {
  return { width: '100%', borderCollapse: 'collapse', fontFamily: font.body };
}
function th(colors: ThemeColors): CSSProperties {
  return { ...type.label, textAlign: 'left', color: colors.textMuted, padding: '6px 10px 8px 0', borderBottom: `1px solid ${colors.border}` };
}
function td(colors: ThemeColors): CSSProperties {
  return { ...type.body, padding: '10px 10px 10px 0', borderBottom: `1px solid ${colors.border}`, verticalAlign: 'top' };
}
function inputStyle(colors: ThemeColors): CSSProperties {
  return {
    ...type.body,
    background: colors.inputBg,
    color: colors.text,
    border: `1px solid ${colors.border}`,
    borderRadius: radius.sm,
    padding: '6px 10px',
    fontFamily: font.body,
    minWidth: 100,
  };
}
function ghostBtn(colors: ThemeColors): CSSProperties {
  return {
    ...type.micro,
    background: 'transparent',
    color: colors.text,
    border: `1px solid ${colors.border}`,
    borderRadius: radius.sm,
    padding: '6px 10px',
    cursor: 'pointer',
    fontFamily: font.body,
  };
}
function ghostBtnOn(colors: ThemeColors): CSSProperties {
  return { ...ghostBtn(colors), borderColor: colors.cyan, color: colors.cyan };
}
function primaryBtn(colors: ThemeColors): CSSProperties {
  return {
    ...type.micro,
    background: colors.cyan,
    color: colors.textOnCyan,
    border: 'none',
    borderRadius: radius.sm,
    padding: '7px 12px',
    cursor: 'pointer',
    fontFamily: font.body,
    fontWeight: 600,
  };
}
