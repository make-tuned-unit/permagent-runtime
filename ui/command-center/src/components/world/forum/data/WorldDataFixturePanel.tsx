/**
 * DEV ONLY — World W1 fixture panel. Not wired into the app shell.
 *
 * Shows each place's interaction schema against each TEST DATA fixture state,
 * so W2 and reviewers can see every evidence state (ready, empty, truncated,
 * stale, error with last good data, unsupported, setup required) without a
 * daemon. Rendered by WorldDataFixturePanel.test.tsx at 1280×800 logical.
 */
import { useState, type CSSProperties } from 'react';
import { Button } from '../../../common/Button';
import { getThemedColors, radius, space } from '../../../../styles/tokens';
import { resolveWorldAgent, WORLD_PLACE_IDS } from './identity';
import { PLACE_SCHEMAS } from './placeSchema';
import { recordLinkLabel } from './recordLinks';
import { sectionFreshness, visibleItems, type Section } from './types';
import type { FixtureRun } from './fixtures/fixtureSnapshots';
import { FIXTURE_MARKER } from './fixtures/worldFixtures';

const COLORS = getThemedColors();
const PANEL: CSSProperties = { width: 1280, height: 800, overflow: 'auto', boxSizing: 'border-box', padding: space.xxl, font: '13px/1.4 system-ui, sans-serif', background: '#f7f5f0', color: '#1d1d1b' };
const GRID: CSSProperties = { display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: space.xl };
const CARD: CSSProperties = { background: '#fff', border: '1px solid #d8d3c8', borderRadius: radius.md, padding: space.lg, minWidth: 0 };

function itemLabel(item: unknown): string {
  const r = item as Record<string, unknown>;
  switch (r.kind) {
    case 'orchestrator': return `${r.name}: ${r.currentState}${r.currentTool ? ` · ${r.currentTool}` : ''} · ${r.tasksInFlight} in flight`;
    case 'goal': return `${r.title} — ${r.phase}`;
    case 'decision': return `${r.headline} (tier ${r.tier})`;
    case 'request': return `${r.description} — ${r.outcome}`;
    case 'memory': {
      const p = r.provenance as { kind: string; tag?: string; origin?: string };
      return `${r.text} · ${p.kind === 'source_tag' ? `source: ${p.tag}` : p.kind === 'key_origin' ? `origin: ${p.origin}` : 'no provenance recorded'}`;
    }
    case 'activity': return `${r.title} (${r.actor})`;
    case 'spend': return `$${(r.costUsd as number).toFixed(2)} · ${r.callCount} calls — ${r.scope}`;
    case 'skill': return `${r.name}${r.usageCount !== null ? ` · used ${r.usageCount}×` : ''}`;
    case 'forecast': {
      const o = r.outcome as { kind: string; methodLabel?: string; reason?: string; points?: number | null; needed?: number | null };
      return `${r.label}: ${o.kind === 'forecast' ? `forecast · ${o.methodLabel}` : `refused · ${o.reason}${o.needed ? ` (${o.points} of ${o.needed} points)` : ''}`}`;
    }
    case 'council_report': return `${r.headline} · ${r.dissentCount} dissent`;
    case 'growth_result': return `${r.title}: ${r.verdict}${r.deltaPct !== null ? ` (${Math.round((r.deltaPct as number) * 100)}%)` : ''}`;
    case 'note': return `${r.title ?? 'Note'}: ${r.body}`;
    case 'agent_state': {
      const t = r.truth as Record<string, { value: string }>;
      return `${r.displayName}: configured ${t.configured.value} · reachable ${t.reachable.value} · ready ${t.ready.value} · used ${t.activelyUsed.value}`;
    }
    case 'sky_provenance': return String(r.statement);
    default: return String(r.kind);
  }
}

function SectionView({ label, section, now, emptyCopy }: { label: string; section: Section<unknown> | undefined; now: number; emptyCopy: string }) {
  if (!section) return <li><strong>{label}</strong>: <em>not loaded</em></li>;
  const fresh = sectionFreshness(section, now);
  const { items, stale, truncated } = visibleItems(section, now);
  let status: string;
  switch (section.state) {
    case 'ready': status = `${items.length} shown${truncated ? ' · truncated, more exist' : ''}${fresh === 'stale' ? ' · stale' : ''}`; break;
    case 'empty': status = `empty — ${section.copy || emptyCopy}`; break;
    case 'error': status = `${section.failure.cause === 'disconnected' ? 'disconnected' : `error (${section.failure.cause})`} — ${section.lastGood ? (section.lastGood.items.length ? `showing last good data from ${section.lastGood.fetchedAt}, stale` : `last known: nothing, as of ${section.lastGood.fetchedAt}, stale`) : 'no earlier data'}`; break;
    case 'unsupported': status = `unsupported — ${section.reason}`; break;
    case 'setup_required': status = `setup required — ${section.reason}`; break;
  }
  return (
    <li data-slot-state={section.state} data-stale={stale ? 'true' : 'false'}>
      <strong>{label}</strong> <span>[{section.state === 'ready' || section.state === 'empty' || section.state === 'error' ? section.contract : '—'}]</span>: {status}
      {items.length > 0 && (
        <ul>
          {items.slice(0, 3).map((item, i) => {
            const link = (item as { link?: Parameters<typeof recordLinkLabel>[0] | null }).link;
            return <li key={i}>{itemLabel(item)}{link ? <> · <u>{recordLinkLabel(link)}</u></> : null}</li>;
          })}
        </ul>
      )}
    </li>
  );
}

export function WorldDataFixturePanel({ runs, initialFixtureId }: { runs: readonly FixtureRun[]; initialFixtureId?: string }) {
  const [selected, setSelected] = useState(initialFixtureId ?? runs[0]?.fixture.id ?? '');
  const run = runs.find((r) => r.fixture.id === selected) ?? runs[0];
  if (!run) return <div style={PANEL}>No fixtures.</div>;
  return (
    <div style={PANEL} data-testid="world-fixture-panel">
      <p role="note" style={{ margin: '0 0 8px', fontWeight: 600, color: '#8a3b00' }}>{FIXTURE_MARKER}</p>
      <div role="group" aria-label="Fixture state" style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm, marginBottom: space.lg }}>
        {runs.map((r) => (
          <Button key={r.fixture.id} colors={COLORS} flashSuccess={false} aria-pressed={r.fixture.id === run.fixture.id} onClick={() => setSelected(r.fixture.id)}>
            {r.fixture.item}. {r.fixture.title}
          </Button>
        ))}
      </div>
      <p style={{ margin: '0 0 10px' }}>{run.fixture.description}{run.fixture.projectId ? ` Project: ${run.fixture.projectId}.` : ' No project chosen.'}</p>
      <div style={GRID}>
        {WORLD_PLACE_IDS.map((placeId) => {
          const schema = PLACE_SCHEMAS.find((p) => p.id === placeId)!;
          const snap = run.snapshots[placeId];
          const host = resolveWorldAgent(schema.host);
          return (
            <section key={placeId} style={CARD} aria-label={schema.accessible.listLabel} data-place={placeId}>
              <h3 style={{ margin: '0 0 4px', fontSize: 15 }}>{schema.name}</h3>
              <div>Host: {schema.host} ({host.status === 'worker' ? host.agentId : host.status}) · {snap.requestCount} request(s) this visit</div>
              <div>Object: {schema.interactionObject}</div>
              <div>Primary: <strong>{schema.primaryAction.label}</strong> [{schema.primaryAction.kind}{schema.primaryAction.sends ? ', sends' : ', does not send'}]</div>
              <div>Also: {schema.secondaryActions.map((a) => `${a.label}${a.sends ? ' (explicit)' : ''}${a.demonstration ? ' (demonstration)' : ''}`).join(' · ')}</div>
              <ul style={{ paddingLeft: space.xxl, margin: '6px 0' }}>
                {schema.evidence.map((slot) => (
                  <SectionView key={slot.id} label={slot.label} section={snap.sections[slot.id] as Section<unknown> | undefined} now={run.now} emptyCopy={slot.emptyCopy} />
                ))}
              </ul>
              <div style={{ color: '#555' }}>Accessible: {schema.accessible.shortcutHint}</div>
            </section>
          );
        })}
      </div>
    </div>
  );
}
