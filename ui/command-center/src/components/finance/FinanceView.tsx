/**
 * Finance — money board.
 *
 * Cards match the rest of the app (Projects Panel: veil, hairline, uppercase
 * 11px title). Polybot and Picker stay off until the user turns them on.
 * Holdings, household, watchlist, and notes are the default surface.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, FormEvent, ReactNode } from 'react';
import { duration, ease, font, radius, tabularNums, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { api, apiFetch, uploadFinanceStatement } from '../../lib/api';
import { ViewHeader } from '../common/ViewHeader';
import { Button, SUCCESS_FLASH_MS } from '../common/Button';
import { navigateToTool } from '../../lib/store';
import { GLOSSARY } from '../../lib/vocabulary';
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
  loopTagLabel,
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

  // Returns whether the action actually succeeded. The error is still shown in
  // the banner, but the boolean is what lets a Button decide between a success
  // tick and no tick — before this, a failed action looked exactly like a
  // successful one because `mutate` swallowed the throw.
  const mutate = useCallback(async (fn: () => Promise<unknown>): Promise<boolean> => {
    setBusy(true);
    try {
      await fn();
      await load();
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Update failed');
      return false;
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
          <Button colors={colors} type="button" onClick={() => load()}>
            Refresh
          </Button>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<boolean>;
}) {
  const p = board.polybot;
  const asOf = p.asOf || p.lastUpdated;
  // Lives here, not in PickerControls, so the "your list: empty" line in the
  // card caption can open the very form it tells the user to use.
  const [showPickerAdd, setShowPickerAdd] = useState(false);
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
            {/* The mechanism around this sentence already escalates correctly —
                warning border on the card, warning tone on the hero figure —
                but the sentence that actually SAYS the balance is 110 days old
                rendered at the same muted caption weight as the routine "Live
                file" line it replaces. The one line carrying the bad news was
                the quietest thing in the card. It now takes small weight and
                the warning tone whenever it is the stale variant, so the words
                and the chrome say the same thing. */}
            <div
              data-testid="finance-polybot-freshness"
              data-stale={p.stale ? 'true' : 'false'}
              style={p.stale
                ? { ...type.small, color: colors.warning, fontWeight: 600, marginTop: 4 }
                : { ...type.caption, color: colors.textMuted, marginTop: 4 }}
            >
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
              value={board.picker.reachable ? (board.picker.scanInProgress ? 'Scanning' : 'Up') : ((board.pickerUniverse?.length ?? 0) ? 'Your list' : 'Idle')}
              tone={board.picker.reachable ? colors.success : colors.text}
            />
            {/* Two different lists used to share the word "universe": the
                scanner's own exchange-listing cache (tens of thousands of
                names it *could* rank) and the handful the user typed in. The
                card showed only the first, labelled as if it were the second.
                They now get separate lines and separate words. */}
            <div data-testid="picker-scanner-pool" style={{ ...type.caption, color: colors.textMuted, marginTop: 4 }}>
              {(board.pickerUniverseCount ?? 0) > 0
                ? `Scanner pool · ${board.pickerUniverseCount?.toLocaleString()} tickers it can rank`
                : board.picker.reachable
                  ? `${board.picker.results != null ? `${board.picker.results} ranked` : 'ready'}${board.picker.scanDate ? ` · ${board.picker.scanDate}` : ''}`
                  : 'Connected when the scanner is up'}
            </div>
            <div
              data-testid="picker-your-tickers"
              style={{ ...type.caption, color: colors.textMuted, marginTop: 2, display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}
            >
              {(board.pickerUniverse?.length ?? 0) > 0
                ? `Your tickers · ${board.pickerUniverse?.length} you added`
                : 'Your tickers · none yet, so picks come from the whole pool'}
              {(board.pickerUniverse?.length ?? 0) === 0 && (
                <Button
                  colors={colors}
                  variant="bare"
                  type="button"
                  data-testid="picker-add-hint"
                  style={{ color: colors.cyan, textDecoration: 'underline', padding: 0 }}
                  onClick={() => setShowPickerAdd(() => true)}
                >
                  Add ticker
                </Button>
              )}
            </div>
            <div style={{ marginTop: 10 }}>
              <PickerControls
                picker={board.picker}
                universe={board.pickerUniverse ?? []}
                colors={colors}
                busy={busy}
                mutate={mutate}
                setLab={setLab}
                showAdd={showPickerAdd}
                setShowAdd={(fn) => setShowPickerAdd(fn)}
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
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<boolean>;
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
          <Button
            colors={colors}
            type="button"
            data-testid="finance-enable-polybot"
            disabled={busy}
            onClick={() => setDialog('polybot')}
          >
            Turn on Polybot
          </Button>
        )}
        {!pickerOn && (
          <Button
            colors={colors}
            type="button"
            data-testid="finance-enable-picker"
            disabled={busy}
            onClick={() => setDialog('picker')}
          >
            Turn on Picker
          </Button>
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
        <Button
          colors={colors}
          variant="primary"
          type="button"
          pending={busy}
          disabled={busy || !checked}
          onClick={onConfirm}
        >
          {confirmLabel}
        </Button>
        <Button colors={colors} type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<boolean>;
}) {
  const [showKeys, setShowKeys] = useState(false);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
        <Button
          colors={colors}
          variant={polybot.running && !polybot.paused ? 'ghost' : 'primary'}
          type="button"
          data-testid="polybot-start"
          disabled={busy || (Boolean(polybot.running) && !polybot.paused)}
          onClick={() => mutate(() =>
            apiFetch('/api/finance/polybot/start', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
        >
          {polybot.running && !polybot.paused ? 'Running' : polybot.paused ? 'Resume' : 'Start'}
        </Button>
        <Button
          colors={colors}
          type="button"
          data-testid="polybot-pause"
          disabled={busy || !polybot.running || polybot.paused}
          onClick={() => mutate(() =>
            apiFetch('/api/finance/polybot/pause', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
        >
          Pause
        </Button>
        <Button
          colors={colors}
          type="button"
          data-testid="polybot-scan"
          disabled={busy || polybot.paused || polybot.scanRequested}
          onClick={() => mutate(() =>
            apiFetch('/api/finance/polybot/scan', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
        >
          {polybot.scanRequested ? 'Scan queued…' : 'Scan now'}
        </Button>
        <Button
          colors={colors}
          type="button"
          aria-expanded={showKeys}
          onClick={() => setShowKeys((v) => !v)}
        >
          {showKeys ? 'Hide keys' : 'Keys'}
        </Button>
        <Button
          colors={colors}
          type="button"
          disabled={busy}
          onClick={() => setLab(POLYBOT_ENABLED_KEY, false)}
        >
          Turn off
        </Button>
      </div>
      {showKeys && (
        <PolybotKeys compact onChanged={() => void mutate(async () => undefined)} />
      )}
    </div>
  );
}

function PickerControls({
  picker, universe, colors, busy, mutate, setLab, showAdd, setShowAdd,
}: {
  picker: PickerStatus;
  universe: string[];
  colors: ThemeColors;
  busy: boolean;
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
  setLab: (key: typeof POLYBOT_ENABLED_KEY | typeof PICKER_ENABLED_KEY, on: boolean) => Promise<boolean>;
  /** Owned by the card so the "Your tickers · none yet" caption can open the
   *  same form its own link points at. */
  showAdd: boolean;
  setShowAdd: (fn: (v: boolean) => boolean) => void;
}) {
  const [draft, setDraft] = useState('');
  const [added, setAdded] = useState(false);

  const saveExtras = (next: string[]) =>
    mutate(() => api.upsertConfig(PICKER_UNIVERSE_KEY, next.join('\n')));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
        <Button
          colors={colors}
          type="button"
          data-testid="picker-start"
          disabled={busy || picker.reachable}
          onClick={() => mutate(() =>
            apiFetch('/api/finance/picker/start', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
        >
          {picker.reachable ? 'Scanner up' : 'Start scanner'}
        </Button>
        <Button
          colors={colors}
          type="button"
          data-testid="picker-scan"
          disabled={busy || picker.scanInProgress}
          onClick={() => mutate(() =>
            apiFetch('/api/finance/picker/scan', { method: 'POST', headers: { 'Content-Type': 'application/json' } }),
          )}
        >
          {picker.scanInProgress ? 'Scan running…' : 'Run scan'}
        </Button>
        <Button
          colors={colors}
          variant={showAdd ? 'ghostOn' : 'ghost'}
          type="button"
          id="picker-add-ticker"
          data-testid="picker-add-ticker"
          aria-expanded={showAdd}
          onClick={() => setShowAdd((v) => !v)}
        >
          {showAdd ? 'Hide add' : 'Add ticker'}
        </Button>
        <Button
          colors={colors}
          type="button"
          disabled={busy}
          onClick={() => setLab(PICKER_ENABLED_KEY, false)}
        >
          Turn off
        </Button>
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
              <Button
                colors={colors}
                variant="bare"
                type="button"
                disabled={busy}
                aria-label={`Remove ${t}`}
                flashSuccess={false}
                onClick={() => saveExtras(universe.filter((x) => x !== t))}
              >
                ×
              </Button>
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
            void saveExtras(next).then((ok) => {
              if (!ok) return;
              setDraft('');
              setAdded(true);
              setTimeout(() => setAdded(false), SUCCESS_FLASH_MS);
            });
          }}
          style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}
        >
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="AAPL, SHOP.TO — names you want ranked"
            style={{ ...inputStyle(colors), flex: 1, minWidth: 140 }}
          />
          <Button
            colors={colors}
            variant="primary"
            type="submit"
            data-testid="picker-universe-add"
            pending={busy}
            success={added}
            disabled={busy || parseUniverse(draft).length === 0}
          >
            Add
          </Button>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
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
        <Button
          colors={colors}
          type="button"
          aria-expanded={showForm || editing}
          onClick={() => { setShowForm((s) => !s); if (editing) setDraft(emptyDraft()); }}
        >
          {showForm || editing ? 'Hide form' : 'Record a trade'}
        </Button>
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
            <Button colors={colors} variant="primary" type="submit" pending={busy} disabled={busy}>
              {editing ? 'Save trade' : 'Record trade'}
            </Button>
            {editing && (
              <Button colors={colors} type="button" disabled={busy} onClick={() => { setDraft(emptyDraft()); setShowForm(false); }}>
                Cancel
              </Button>
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
            <Button colors={colors} variant={filter === 'open' ? 'ghostOn' : 'ghost'} type="button" aria-pressed={filter === 'open'} onClick={() => setFilter('open')}>
              Open ({holdings.openCount})
            </Button>
            <Button colors={colors} variant={filter === 'all' ? 'ghostOn' : 'ghost'} type="button" aria-pressed={filter === 'all'} onClick={() => setFilter('all')}>
              All ({holdings.rows.length})
            </Button>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
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
        <Button colors={colors} type="button" disabled={busy} onClick={onEdit}>Edit</Button>
        {!row.exitDate && (
          <Button colors={colors} type="button" disabled={busy} aria-expanded={closing} onClick={() => setClosing((c) => !c)}>
            Close
          </Button>
        )}
        <Button
          colors={colors}
          type="button"
          disabled={busy}
          onClick={() => mutate(() => apiFetch(deletePath, { method: 'DELETE' }))}
        >
          Remove
        </Button>
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
          <Button colors={colors} variant="primary" type="submit" pending={busy} disabled={busy}>Mark closed</Button>
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
        {/* A bare agent name is not a label — it says who, never what pressing
            it does. The app already gets this right elsewhere ("Security — from
            the Guard"), so the cross-link takes a verb and names its
            destination. */}
        <Button
          colors={colors}
          type="button"
          data-testid="picks-world-link"
          title="Open the World view, where the Financier's panel lives"
          onClick={() => navigateToTool('world')}
        >
          View in World
        </Button>
      </div>
      {/* The old caption said "your universe" even when the user had added
          nothing — in that case every row is the scanner's own ranking, so say
          so. The second line is the pipeline in one sentence: it is the only
          place the words on the tags below get defined.

          The badge sentence names the badge's own words. It used to say "Gold
          means…", which described the mark by its colour — across three themes,
          on a tag that has said "Agent approved" in words since it was
          rewritten. Naming a control by a colour only works for readers who see
          that colour the same way, and only until the colour changes. */}
      <p style={{ ...type.caption, color: colors.textMuted, margin: '0 0 4px' }} data-testid="picks-caption">
        {(board.pickerUniverse?.length ?? 0) > 0
          ? 'Your tickers plus the scanner’s own ranking, priced from Yahoo.'
          : 'The scanner’s own ranking — you haven’t added tickers of your own yet.'}
        {' '}An “Agent approved” tag means the Financier approved that pick.
      </p>
      <p style={{ ...type.caption, color: colors.textMuted, margin: '0 0 10px', opacity: 0.85 }} data-testid="picks-legend">
        How a name gets here: scanner ranks it {'→'} Yahoo prices it {'→'} a
        significance check asks whether the signal is real {'→'} the Financier
        picks at most one. Names that fail the check stay listed, tagged in red {'—'} click a
        tag to see why.
      </p>
      {ranked.length === 0 ? (
        <p style={{ ...type.small, color: colors.textMuted, margin: 0 }}>
          {(board.pickerUniverse?.length ?? 0) === 0 && !(board.pickerUniverseCount)
            ? 'Add tickers of your own, or run a scan.'
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
            <Button
              colors={colors}
              type="button"
              style={{ alignSelf: 'flex-start', marginTop: 8 }}
              onClick={() => setShowAll(true)}
            >
              Show {hidden} more
            </Button>
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
  const rowId = `pick-detail-${pick.ticker.replace(/[^A-Za-z0-9]/g, '-')}`;
  const loopTitle = loop
    ? loop.passed
      ? 'This name passed the significance check.'
      : `Why it was filtered: ${loop.kills.join('; ')}`
    : undefined;
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
          aria-expanded={open}
          aria-controls={rowId}
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
          {/* Nothing said this row opened. A chevron is the cheapest way to
              say it, and it rotates so open/closed reads at a glance. */}
          <span
            aria-hidden="true"
            style={{
              ...type.micro,
              color: colors.textMuted,
              display: 'inline-block',
              transform: open ? 'rotate(90deg)' : 'none',
              transition: `transform ${duration.fast}ms ${ease.out}`,
            }}
          >
            ▸
          </span>
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
            title={GLOSSARY.financierApproved}
            style={{
              ...type.micro,
              color: '#3d2e0a',
              background: AGENT_TRIM.financier,
              fontWeight: 700,
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
              padding: '2px 7px',
              borderRadius: radius.pill,
            }}
          >
            Agent approved
          </span>
        )}
        {loop && (
          // The tag IS the affordance: the reason it names is one click away,
          // and hovering shows the full sentence without a click at all.
          <button
            type="button"
            data-testid="pick-loop-tag"
            data-passed={loop.passed ? 'true' : 'false'}
            aria-expanded={open}
            aria-controls={rowId}
            title={loopTitle}
            onClick={() => setOpen((v) => !v)}
            style={{
              ...type.micro,
              color: loop.passed ? colors.success : colors.danger,
              fontWeight: 600,
              fontFamily: font.body,
              background: warnFill(loop.passed ? colors.success : colors.danger, 0.10),
              border: `1px solid ${warnFill(loop.passed ? colors.success : colors.danger, 0.35)}`,
              borderRadius: radius.pill,
              padding: '1px 8px',
              cursor: 'pointer',
              whiteSpace: 'nowrap',
            }}
          >
            {loopTagLabel(loop)}
            {!loop.passed && <span aria-hidden="true" style={{ opacity: 0.7 }}> ⓘ</span>}
          </button>
        )}
        <span style={{ ...type.caption, color: colors.textMuted, ...tabularNums }}>
          {fmtMoney(yahoo ?? pick.pickerPrice, pick.quote?.currency ?? undefined)}
        </span>
        <Button
          colors={colors}
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
        >
          Prefill
        </Button>
      </div>
      {open && (
        <div id={rowId} style={{ ...type.caption, color: colors.textMuted, marginTop: 6, display: 'flex', flexDirection: 'column', gap: 4 }}>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', ...tabularNums }}>
            <span>Scan {fmtMoney(pick.pickerPrice)}</span>
            <span>Yahoo {pick.quoteError ? pick.quoteError : fmtMoney(yahoo, pick.quote?.currency ?? undefined)}</span>
            <span>RSI {pick.pickerRsi != null ? pick.pickerRsi.toFixed(1) : '—'}</span>
            {pick.score != null && <span>Score {pick.score.toFixed(1)}</span>}
          </div>
          {pick.reason && <p style={{ margin: 0 }}>{pick.reason}</p>}
          {loop && (
            <p style={{ margin: 0 }} data-testid="pick-loop-detail">
              {loop.kills.length > 0
                ? `Why it was filtered: ${loop.kills.join('; ')}. `
                : 'Passed the significance check. '}
              {/* Two borrowed terms, defined where they appear. They were bare
                  chrome: a reader who does not already know what an ICIR is
                  learns nothing from a number beside it, and the number is
                  precisely what the filter above it acted on. */}
              <span style={{ opacity: 0.75 }} data-testid="pick-loop-metrics">
                <span title={GLOSSARY.icir} style={{ cursor: 'help', borderBottom: `1px dotted ${colors.border}` }}>
                  ICIR
                </span>
                {' '}{fmtNum(loop.icir)} ·{' '}
                <span title={GLOSSARY.halfLife} style={{ cursor: 'help', borderBottom: `1px dotted ${colors.border}` }}>
                  half-life
                </span>
                {' '}{loop.halfLifeDays != null ? `${loop.halfLifeDays.toFixed(1)}d` : '—'}
              </span>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
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
        <Button colors={colors} type="button" disabled={busy} onClick={() => fileRef.current?.click()}>
          Choose file
        </Button>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
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
                    <Button
                      colors={colors}
                      type="button"
                      disabled={busy}
                      flashSuccess={false}
                      onClick={() => mutate(() =>
                        apiFetch(`/api/finance/watchlist/${encodeURIComponent(row.symbol)}`, { method: 'DELETE' }),
                      )}
                    >
                      Remove
                    </Button>
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
        <Button colors={colors} type="submit" pending={busy} disabled={busy || !symbol.trim()}>Add</Button>
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
  mutate: (fn: () => Promise<unknown>) => Promise<boolean>;
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
              <Button
                colors={colors}
                type="button"
                disabled={busy}
                flashSuccess={false}
                onClick={() => mutate(() =>
                  apiFetch(`/api/finance/notes/${encodeURIComponent(n.id)}`, { method: 'DELETE' }),
                )}
              >
                Delete
              </Button>
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
        <Button colors={colors} type="submit" pending={busy} disabled={busy} style={{ alignSelf: 'flex-start' }}>Add note</Button>
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

