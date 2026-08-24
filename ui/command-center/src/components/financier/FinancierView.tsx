/**
 * The Financier — the money tab.
 *
 * ## What is on it, and why only this
 *
 * Everything here reads a source that already exists. Nothing is seeded,
 * mocked, or padded:
 *
 *   * **Spend** — `GET /api/governance/spend` over the real `cost_ledger`.
 *     This panel was Settings → Spend and now lives here; it was not copied.
 *     Settings no longer has a Spend section, so there is one place to see
 *     what the machine is costing and one place to set the ceilings.
 *   * **Where its work WOULD run** — `GET /api/finance/routing`, the live
 *     answer from `financier::resolve_live_financier_route`. Read carefully:
 *     that resolver states the local-first rule and reports whether this
 *     machine can satisfy it; it does not steer dispatch yet, and the card
 *     says so on screen.
 *   * **Market read** — `GET /api/finance/quote`, straight through to Yahoo's
 *     public endpoint. Nothing is cached and nothing is stored.
 *   * **What has left this machine** — the existing egress audit log, narrowed
 *     to the `market_data` rows this tab's own reads produce.
 *
 * ## What is deliberately NOT on it
 *
 * There is no holdings table, no positions, no net worth, no account balances
 * and no imported statements, because this codebase stores none of those
 * things — there is no table, no migration and no importer for any of them.
 * A panel with plausible-looking rows would be a lie about the user's money,
 * which is the worst thing this surface could do. The empty state below says
 * so in as many words instead.
 *
 * Statement ingestion is the obvious next step and it is deliberately out of
 * scope: it depends on the downloads/inbox front door, which is unfinished.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  api,
  type EgressLogEntry,
  type FinancierRouting,
  type Quote,
} from '../../lib/api';
import { font, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Card, SectionLabel } from '../settings/atoms';
import { timeAgo } from '../settings/format';
import { SpendPanel } from './SpendPanel';

/** The platform-extension key that switches the Financier on. It is the same
 *  string the daemon registry uses (`finance::EXTENSION_NAME`) and the same one
 *  the Agents surface reports a capability under. */
const FINANCE_EXTENSION_KEY = 'finance';

/** The egress-audit `kind` written by a market-data read
 *  (`sovereignty::EgressKind::MarketData`). */
const MARKET_DATA_KIND = 'market_data';

/**
 * A number the source did not report renders as an em dash, never as 0.
 * Every numeric field on `Quote` is optional precisely so this distinction
 * survives to the screen.
 */
function num(value: number | null | undefined, digits = 2): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? value.toLocaleString(undefined, {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
      })
    : '—';
}

function QuoteCard({ quote }: { quote: Quote }) {
  const { colors } = useTheme();
  const up = typeof quote.change === 'number' && quote.change >= 0;
  const moved = typeof quote.change === 'number';
  return (
    <Card>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ fontFamily: font.display, fontSize: 18, fontWeight: 600 }}>
          {quote.symbol}
        </span>
        {quote.name && (
          <span style={{ fontSize: 12, color: colors.textMuted }}>{quote.name}</span>
        )}
        {quote.market_closed && (
          <span style={{ fontSize: 11, color: colors.textDim }}>market closed</span>
        )}
      </div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, marginTop: 10 }}>
        <span style={{ fontSize: 26, fontWeight: 600, ...tabularNums }}>
          {num(quote.price)}
        </span>
        {quote.currency && (
          <span style={{ fontSize: 12, color: colors.textMuted }}>{quote.currency}</span>
        )}
        {moved && (
          <span style={{ fontSize: 13, color: up ? colors.success : colors.danger, ...tabularNums }}>
            {up ? '+' : ''}{num(quote.change)} ({up ? '+' : ''}{num(quote.change_percent)}%)
          </span>
        )}
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))', gap: 10, marginTop: 16 }}>
        {[
          ['Previous close', num(quote.previous_close)],
          ['Day range', `${num(quote.day_low)} – ${num(quote.day_high)}`],
          ['52-week range', `${num(quote.fifty_two_week_low)} – ${num(quote.fifty_two_week_high)}`],
          ['Volume', quote.volume == null ? '—' : quote.volume.toLocaleString()],
        ].map(([label, value]) => (
          <div key={label}>
            <div style={{ fontSize: 11, color: colors.textMuted }}>{label}</div>
            <div style={{ fontSize: 13, marginTop: 2, ...tabularNums }}>{value}</div>
          </div>
        ))}
      </div>
      <div style={{ fontSize: 11, color: colors.textDim, marginTop: 14 }}>
        {/* The timestamp is not decoration: it is the only thing that makes the
            number above meaningful, and it comes from the exchange, not from
            when this component rendered. */}
        {quote.quoted_at
          ? `Quoted ${timeAgo(quote.quoted_at)} · read live, not stored`
          : 'The source gave no timestamp for this reading.'}
      </div>
    </Card>
  );
}

/** Where the Financier's inference WOULD run, and the one control over it. */
function RoutingCard({
  routing,
  onConsentChanged,
}: {
  routing: FinancierRouting;
  onConsentChanged: () => Promise<void>;
}) {
  const { colors } = useTheme();
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<boolean | null>(null);
  const allowed = pending ?? routing.cloud_allowed;

  const save = async (value: boolean) => {
    setPending(value);
    setError(null);
    try {
      await api.upsertConfig(routing.cloud_consent_key, value);
    } catch (err) {
      // A write that failed is never shown as a success.
      setPending(null);
      setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    try {
      await onConsentChanged();
    } catch (err) {
      setError(`Saved, but could not re-read it: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setPending(null);
    }
  };

  return (
    <Card>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span
          style={{
            fontSize: 11, fontWeight: 600, padding: '3px 10px', borderRadius: 999,
            background: routing.is_local ? colors.cyanSoft : `${colors.danger}1A`,
            color: routing.is_local ? colors.cyan : colors.danger,
          }}
        >
          {routing.is_local ? 'on this machine' : 'cloud'}
        </span>
        {routing.provider && (
          <span style={{ fontSize: 12, color: colors.textMuted, fontFamily: font.mono }}>
            {routing.provider}{routing.model ? `/${routing.model}` : ''}
          </span>
        )}
      </div>
      <div style={{ fontSize: 13, color: colors.text, marginTop: 12, lineHeight: 1.6, maxWidth: 620 }}>
        {routing.statement}
      </div>
      <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 10, lineHeight: 1.55, maxWidth: 620 }}>
        {/* The one claim on this tab most worth being pedantic about. The rule
            below is resolved live and it is real, but nothing dispatches on it
            yet: a Financier tool running inside a session is still served by
            that session's provider. Saying "runs locally" here would be the
            exact overclaim this surface exists to avoid. */}
        This is the rule and whether your machine can satisfy it — not a report
        of where a call went. Model calls made by the Financier's tools inside a
        chat are still served by that session's provider; choosing a provider
        per worker is not built yet. Market data reads are separate, and those{' '}
        <em>are</em> enforced: every one is audited below and refused outright
        under sovereign mode.
      </div>
      <label
        style={{
          display: 'flex', alignItems: 'flex-start', gap: 10, marginTop: 18,
          paddingTop: 16, borderTop: `1px solid ${colors.border}`, cursor: 'pointer',
        }}
      >
        <input
          type="checkbox"
          checked={allowed}
          onChange={e => { void save(e.target.checked); }}
          style={{ marginTop: 3 }}
        />
        <span>
          <span style={{ fontSize: 13, color: colors.text }}>
            Let the Financier use a cloud model when no local one is available
          </span>
          <span style={{ display: 'block', fontSize: 11, color: colors.textMuted, marginTop: 4, lineHeight: 1.55, maxWidth: 560 }}>
            {/* Named honestly: this is a standing permission, not a prompt per
                call. Describing it as per-call consent would be the kind of
                overclaim this tab exists to avoid. */}
            Off by default. This is a standing permission you set once — it does
            not ask again per request. Every cloud call is recorded in the egress
            audit below either way. Writes <code style={{ fontFamily: font.mono }}>{routing.cloud_consent_key}</code>.
          </span>
        </span>
      </label>
      {error && <div style={{ marginTop: 10, fontSize: 12, color: colors.danger }}>{error}</div>}
    </Card>
  );
}

export function FinancierView() {
  const { colors } = useTheme();
  const [routing, setRouting] = useState<FinancierRouting | null>(null);
  const [routingError, setRoutingError] = useState<string | null>(null);
  const [egress, setEgress] = useState<EgressLogEntry[] | null>(null);

  const [symbol, setSymbol] = useState('');
  const [quote, setQuote] = useState<Quote | null>(null);
  const [quoteError, setQuoteError] = useState<string | null>(null);
  const [loadingQuote, setLoadingQuote] = useState(false);

  const [enabling, setEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string | null>(null);

  const loadRouting = useCallback(async () => {
    try {
      setRouting(await api.getFinancierRouting());
      setRoutingError(null);
    } catch (err) {
      // The routing readout is the one claim this tab makes about where data
      // goes. If it cannot be read, say so — never fall back to assuming local.
      setRouting(null);
      setRoutingError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const loadEgress = useCallback(async () => {
    try {
      setEgress(await api.getEgressLog(200));
    } catch {
      setEgress(null);
    }
  }, []);

  useEffect(() => { void loadRouting(); void loadEgress(); }, [loadRouting, loadEgress]);

  const marketRows = useMemo(
    () => (egress ?? []).filter(row => row.kind === MARKET_DATA_KIND),
    [egress],
  );

  const lookUp = useCallback(async () => {
    const trimmed = symbol.trim();
    if (!trimmed) return;
    setLoadingQuote(true);
    setQuoteError(null);
    setQuote(null);
    try {
      setQuote(await api.getQuote(trimmed));
    } catch (err) {
      // The reason is shown verbatim: a sovereign-mode refusal and an
      // unreachable source need different fixes, and a single "could not load"
      // would hide which one happened.
      setQuoteError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoadingQuote(false);
      void loadEgress();
    }
  }, [symbol, loadEgress]);

  const enable = useCallback(async () => {
    setEnabling(true);
    setEnableError(null);
    try {
      await api.setExtensionEnabled(FINANCE_EXTENSION_KEY, true);
      await loadRouting();
    } catch (err) {
      setEnableError(err instanceof Error ? err.message : String(err));
    } finally {
      setEnabling(false);
    }
  }, [loadRouting]);

  return (
    <div style={{ height: '100%', overflowY: 'auto', padding: '32px 36px', fontFamily: font.body, color: colors.text }}>
      <div style={{ maxWidth: 900, margin: '0 auto' }}>
        <div style={{ marginBottom: 28 }}>
          <div style={{ fontFamily: font.display, fontSize: 24, fontWeight: 600, letterSpacing: '-0.02em' }}>
            The Financier
          </div>
          <div style={{ fontSize: 13, color: colors.textMuted, marginTop: 6, maxWidth: 620, lineHeight: 1.55 }}>
            Money — what your own AI work costs, and live market reads. It reports
            numbers; it cannot place an order, and there is no tool anywhere in it
            that can.
          </div>
        </div>

        {routing && !routing.enabled && (
          <Card>
            <SectionLabel>The Financier is switched off</SectionLabel>
            <div style={{ fontSize: 13, color: colors.textMuted, marginTop: 8, lineHeight: 1.6, maxWidth: 620 }}>
              Its tools are not loaded into new sessions, so the agent cannot use
              them. Spend below still reads normally — that is the cost ledger,
              which records regardless.
            </div>
            <button
              onClick={() => { void enable(); }}
              disabled={enabling}
              style={{
                marginTop: 14, padding: '8px 14px', borderRadius: 8, fontSize: 13,
                background: colors.cyanSoft, color: colors.cyan,
                border: `1px solid ${colors.borderHi}`,
                cursor: enabling ? 'wait' : 'pointer',
              }}
            >
              {enabling ? 'Enabling…' : 'Enable the Financier'}
            </button>
            {enableError && (
              <div style={{ marginTop: 10, fontSize: 12, color: colors.danger }}>{enableError}</div>
            )}
          </Card>
        )}

        <SectionLabel>Where its work would run</SectionLabel>
        {routingError ? (
          <Card>
            <div style={{ fontSize: 13, color: colors.danger }}>
              Could not read the routing decision: {routingError}
            </div>
            <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 8, lineHeight: 1.55 }}>
              Until this loads, nothing here can tell you whether the Financier
              would run locally, so it is not claiming that it would.
            </div>
          </Card>
        ) : routing ? (
          <RoutingCard routing={routing} onConsentChanged={loadRouting} />
        ) : (
          <Card><div style={{ fontSize: 13, color: colors.textDim }}>Reading…</div></Card>
        )}

        <SectionLabel>Market read</SectionLabel>
        <Card>
          <div style={{ fontSize: 12, color: colors.textMuted, marginBottom: 12, lineHeight: 1.55, maxWidth: 620 }}>
            One symbol at a time, read live from the public market-data endpoint.
            Nothing is cached and nothing is saved — no watchlist, no history.
            The symbol travels to the data source to fetch anything at all, so
            every read is recorded in the audit below.
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <input
              value={symbol}
              onChange={e => setSymbol(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter') void lookUp(); }}
              placeholder="Symbol, e.g. a ticker or an index"
              style={{
                flex: 1, padding: '8px 12px', background: colors.inputBg,
                border: `1px solid ${colors.border}`, borderRadius: 8,
                color: colors.text, fontFamily: font.mono, fontSize: 13, outline: 'none',
              }}
            />
            <button
              onClick={() => { void lookUp(); }}
              disabled={loadingQuote || symbol.trim() === ''}
              style={{
                padding: '8px 16px', borderRadius: 8, fontSize: 13,
                background: colors.surfaceHi, color: colors.text,
                border: `1px solid ${colors.border}`,
                cursor: loadingQuote || symbol.trim() === '' ? 'not-allowed' : 'pointer',
                opacity: loadingQuote || symbol.trim() === '' ? 0.55 : 1,
              }}
            >
              {loadingQuote ? 'Reading…' : 'Read'}
            </button>
          </div>
          {quoteError && (
            <div style={{ marginTop: 12, fontSize: 12, color: colors.danger, lineHeight: 1.55 }}>
              {quoteError}
            </div>
          )}
        </Card>
        {quote && <QuoteCard quote={quote} />}

        <SectionLabel>What your AI work costs</SectionLabel>
        {/* Moved here from Settings → Spend, not duplicated: that settings
            section no longer exists. Same endpoints, same ledger, one home. */}
        <SpendPanel />

        <SectionLabel>Market reads that left this machine</SectionLabel>
        <Card>
          {egress === null ? (
            <div style={{ fontSize: 13, color: colors.textDim }}>
              Could not read the egress audit log.
            </div>
          ) : marketRows.length === 0 ? (
            <div style={{ fontSize: 13, color: colors.textMuted, lineHeight: 1.6 }}>
              No market data has been fetched from this machine yet. This list is
              the audit log itself, not a summary of it — a quiet list here means
              no market read happened, but note that the audit covers inference
              and market reads, so it is not proof that nothing at all left.
            </div>
          ) : (
            <div style={{ display: 'grid', gap: 8 }}>
              {marketRows.slice(0, 25).map(row => (
                <div
                  key={row.id}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap',
                    fontSize: 12, paddingBottom: 8, borderBottom: `1px solid ${colors.border}`,
                  }}
                >
                  <span style={{ fontFamily: font.mono, color: colors.text }}>{row.model}</span>
                  <span style={{ color: colors.textMuted, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {row.provider}
                  </span>
                  <span style={{ color: row.blocked ? colors.danger : colors.textDim }}>
                    {row.blocked ? 'blocked' : 'sent'}
                  </span>
                  <span style={{ color: colors.textDim }}>{timeAgo(row.ts)}</span>
                </div>
              ))}
            </div>
          )}
        </Card>

        <SectionLabel>Holdings, accounts and statements</SectionLabel>
        <Card>
          <div style={{ fontSize: 13, color: colors.textMuted, lineHeight: 1.7, maxWidth: 620 }}>
            Permagent stores none of these. There is no table, no import path and
            no connection to a bank or a brokerage anywhere in it, so this tab
            cannot show you a portfolio, a balance or a net-worth figure — and it
            will not show you an invented one.
            <br /><br />
            Reading statements is the intended next step. It waits on the
            downloads inbox, which is not finished, and is deliberately not
            started here.
          </div>
        </Card>
      </div>
    </div>
  );
}
