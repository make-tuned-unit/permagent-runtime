/**
 * Grow → Results: what the actions you took actually did, here and across
 * every active project.
 *
 * Tracking lives on the Actions lens as a per-card rail. That is the right
 * place to watch one experiment; it is the wrong place to answer "are the
 * growth moves paying off?" — there is no summary, and nothing from other
 * projects. This lens is that summary, from `growth_action_outcomes`.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { Button } from '../common/Button';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { usePollWhenVisible } from '../../lib/usePollWhenVisible';
import { useToolOnScreen } from '../../lib/useToolOnScreen';
import { AsOf } from '../common/AsOf';
import type { Project } from '../projects/types';
import { ACTION_CATEGORY_LABELS, normalizeActionCategory } from './growActionTabs';
import { GrowthSparkline } from './GrowthSparkline';
import {
  formatDeltaPct,
  type GrowthCategorySummary,
  type GrowthFleetResults,
  type GrowthResultRow,
  type GrowthResultsData,
} from './growthResults';

type LoadState = 'loading' | 'ready' | 'error';

function verdictLabel(verdict: string | null, colors: ThemeColors): { label: string; color: string } {
  switch (verdict) {
    case 'helped': return { label: 'Helped', color: colors.success };
    case 'hindered': return { label: 'Hindered', color: colors.danger };
    case 'no_effect': return { label: 'No detectable change', color: colors.textDim };
    case 'confounded': return { label: 'Overlapped another change', color: colors.textDim };
    default: return { label: 'Not enough data yet', color: colors.textMuted };
  }
}

function statusLabel(status: string): string {
  switch (status) {
    case 'done': return 'Shipped, awaiting a check';
    case 'verified':
    case 'measuring': return 'Being measured';
    case 'judged': return 'Measured';
    default: return status;
  }
}

/** Matches GrowView's VERDICT_POLL_MS — one cadence for one fact. */
const RESULTS_POLL_MS = 120_000;

export function GrowResults({ project, colors }: { project: Project; colors: ThemeColors }) {
  const [data, setData] = useState<GrowthResultsData | null>(null);
  const [state, setState] = useState<LoadState>('loading');
  /** When these verdicts were last confirmed true. */
  const [asOf, setAsOf] = useState<number | null>(null);
  const projectsRev = useCommandCenter((s) => s.projectsRev);

  /** `silent` keeps a background re-read from throwing the lens back to its
   *  loading copy: a refresh must not look like a first load. A failed silent
   *  read also keeps the last good verdicts rather than blanking them — but it
   *  leaves `asOf` where it was, so nothing stale reads as fresh. */
  const load = useCallback((id: string, opts?: { silent?: boolean }) => {
    if (!opts?.silent) setState('loading');
    apiFetch<GrowthResultsData>(
      `/api/growth-results?projectId=${encodeURIComponent(id)}`,
    )
      .then((d) => {
        setData(d);
        setState('ready');
        setAsOf(Date.now());
      })
      .catch(() => {
        if (opts?.silent) return;
        setData(null);
        setState('error');
      });
  }, []);

  useEffect(() => {
    load(project.id);
  }, [project.id, load, projectsRev]);

  // R1.4: the nightly sweep writes these verdicts and emits nothing, so
  // `projectsRev` above never fires for them. Slow poll while this lens is the
  // surface on screen — see VERDICT_POLL_MS in GrowView for why it is slow.
  const onScreen = useToolOnScreen('grow');
  usePollWhenVisible(() => load(project.id, { silent: true }), RESULTS_POLL_MS, onScreen);

  if (state === 'loading' && !data) {
    return <p style={{ fontSize: textSize.small, color: colors.textDim }}>Loading results…</p>;
  }
  if (state === 'error' && !data) {
    return (
      <p style={{ fontSize: textSize.small, color: colors.danger }}>
        Could not load growth results.{' '}
        <Button
          colors={colors}
          variant="bare"
          onClick={() => load(project.id)}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-fg-hover': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 400,
            fontSize: 'inherit',
            lineHeight: 'inherit',
            textDecoration: 'underline',
          } as CSSProperties}
        >Try again</Button>
      </p>
    );
  }

  const projectResults = data?.project ?? null;
  const fleet = data?.fleet;

  return (
    <section>
      <div style={{ marginBottom: 18 }}>
        <h3 style={{
          fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim,
          textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 4px',
        }}>This project</h3>
        <p style={{ fontSize: textSize.small, color: colors.textMuted, margin: 0, lineHeight: 1.5 }}>
          Actions you marked implemented, and what the 7 / 14 / 28-day windows
          have said so far{projectResults?.segmentLabel ? ` — ${projectResults.segmentLabel}` : ''}.
        </p>
        {/* The windows are judged by a nightly sweep, so "when did this last
            change" is a real question here in a way it is not on a live board.
            Quiet while fresh; it speaks up once the reading has aged past two
            poll intervals. */}
        <p style={{ fontSize: textSize.micro, color: colors.textDim, margin: '6px 0 0' }}>
          <AsOf asOf={asOf} prefix="Verdicts read" staleAfterMs={RESULTS_POLL_MS * 2} dot />
        </p>
      </div>

      {projectResults && projectResults.implemented > 0 ? (
        <>
          <TallyRow
            colors={colors}
            items={[
              { label: 'Taken', value: projectResults.implemented },
              { label: 'Measuring', value: projectResults.measuring },
              { label: 'Helped', value: projectResults.helped, tint: colors.success },
              { label: 'Hindered', value: projectResults.hindered, tint: colors.danger },
              { label: 'No change', value: projectResults.noEffect },
              { label: 'Too little data', value: projectResults.inconclusive },
            ]}
          />
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 12 }}>
            {projectResults.actions.map((row) => (
              <ResultRowCard key={row.actionId} row={row} colors={colors} showProject={false} />
            ))}
          </div>
        </>
      ) : (
        <p style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5, margin: 0 }}>
          No growth actions have been marked implemented on this project yet.
          Send one to a coding agent from Actions, or verify it after you ship
          — then the 7, 14 and 28-day windows land here.
        </p>
      )}

      {fleet && (
        <FleetSection fleet={fleet} colors={colors} />
      )}
    </section>
  );
}

function FleetSection({ fleet, colors }: { fleet: GrowthFleetResults; colors: ThemeColors }) {
  const measured = fleet.helped + fleet.hindered + fleet.noEffect + fleet.inconclusive;
  return (
    <div style={{ marginTop: 28 }}>
      <h3 style={{
        fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim,
        textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 4px',
      }}>Across all projects</h3>
      <p style={{ fontSize: textSize.small, color: colors.textMuted, margin: '0 0 12px', lineHeight: 1.5 }}>
        {measured === 0
          ? 'Once any project has a measured window, the same strategies show up here so a result on one site can inform the next.'
          : `Measured on ${fleet.projects} active project${fleet.projects === 1 ? '' : 's'}. Aggregate and per-category, never merged into one score.`}
      </p>
      {measured > 0 && (
        <>
          <TallyRow
            colors={colors}
            items={[
              { label: 'Projects', value: fleet.projects },
              { label: 'Helped', value: fleet.helped, tint: colors.success },
              { label: 'Hindered', value: fleet.hindered, tint: colors.danger },
              { label: 'No change', value: fleet.noEffect },
              { label: 'Too little data', value: fleet.inconclusive },
            ]}
          />
          {(fleet.trend?.length ?? 0) > 0 && (
            <div style={{ marginTop: 14 }}>
              <h4 style={{
                fontFamily: font.mono, fontSize: 10, color: colors.textDim,
                textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 6px',
              }}>Last 12 weeks — cumulative helped minus hindered</h4>
              <GrowthSparkline points={fleet.trend} colors={colors} height={72} />
            </div>
          )}
          {(fleet.byProject?.length ?? 0) > 0 && (
            <div style={{ marginTop: 14 }}>
              <h4 style={{
                fontFamily: font.mono, fontSize: 10, color: colors.textDim,
                textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 8px',
              }}>By project</h4>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {fleet.byProject.map((row) => (
                  <div
                    key={row.projectId}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 12,
                      background: colors.surface, border: `1px solid ${colors.border}`,
                      borderRadius: radius.md, padding: '8px 12px',
                    }}
                  >
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 600, color: colors.text }}>
                        {row.projectName}
                      </div>
                      <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginTop: 2 }}>
                        {row.helped} helped · {row.hindered} hindered
                        {row.noEffect > 0 ? ` · ${row.noEffect} no change` : ''}
                      </div>
                    </div>
                    <div style={{ width: 140, flexShrink: 0 }}>
                      <GrowthSparkline points={row.points} colors={colors} height={28} showAxis={false} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
          {fleet.categories.length > 0 && (
            <div style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))',
              gap: 10,
              marginTop: 14,
            }}>
              {fleet.categories.map((cat) => (
                <CategoryChip key={cat.category} cat={cat} colors={colors} />
              ))}
            </div>
          )}
          {fleet.recent.length > 0 && (
            <div style={{ marginTop: 18 }}>
              <h4 style={{
                fontFamily: font.mono, fontSize: 10, color: colors.textDim,
                textTransform: 'uppercase', letterSpacing: '0.08em', margin: '0 0 8px',
              }}>Recent verdicts</h4>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {fleet.recent.map((row) => (
                  <ResultRowCard key={`${row.projectId}-${row.actionId}`} row={row} colors={colors} showProject />
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function TallyRow({
  colors, items,
}: {
  colors: ThemeColors;
  items: { label: string; value: number; tint?: string }[];
}) {
  return (
    <div style={{
      display: 'flex', flexWrap: 'wrap', gap: 8,
    }}>
      {items.map((item) => (
        <div
          key={item.label}
          style={{
            background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: '8px 12px', minWidth: 88,
          }}
        >
          <div style={{
            fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
            textTransform: 'uppercase', color: colors.textDim, marginBottom: 2,
          }}>{item.label}</div>
          <div style={{
            fontFamily: font.display, fontSize: textSize.title, fontWeight: 600,
            color: item.tint ?? colors.text, fontVariantNumeric: 'tabular-nums',
          }}>{item.value}</div>
        </div>
      ))}
    </div>
  );
}

function CategoryChip({ cat, colors }: { cat: GrowthCategorySummary; colors: ThemeColors }) {
  const key = normalizeActionCategory(cat.category);
  const label = ACTION_CATEGORY_LABELS[key] ?? cat.category;
  const delta = formatDeltaPct(cat.medianDeltaPct);
  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.md, padding: 12,
    }}>
      <div style={{
        fontFamily: font.mono, fontSize: 10, letterSpacing: '0.08em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: 6,
      }}>{label}</div>
      <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.45 }}>
        Helped {cat.helped} · hindered {cat.hindered} · no change {cat.noEffect}
        {cat.projects > 0 ? ` · ${cat.projects} project${cat.projects === 1 ? '' : 's'}` : ''}
        {delta ? ` · median ${delta}` : ''}
      </div>
    </div>
  );
}

export function ResultRowCard({
  row, colors, showProject,
}: {
  row: GrowthResultRow;
  colors: ThemeColors;
  showProject: boolean;
}) {
  const v = verdictLabel(row.verdict, colors);
  const delta = formatDeltaPct(row.deltaPct);
  const cat = ACTION_CATEGORY_LABELS[normalizeActionCategory(row.category)] ?? row.category;
  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: '12px 14px',
    }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
        <span style={{
          fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
          textTransform: 'uppercase', color: colors.textDim,
        }}>{cat}</span>
        <span style={{ fontFamily: font.display, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>
          {row.title}
        </span>
        <div style={{ flex: 1 }} />
        <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: v.color }}>
          {row.verdict ? v.label : statusLabel(row.status)}
          {delta ? ` · ${delta}` : ''}
          {row.windowDays ? ` at ${row.windowDays}d` : ''}
        </span>
      </div>
      <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: 4 }}>
        {showProject ? `${row.projectName} · ` : ''}
        {row.targetMetric && row.targetDir
          ? `${row.targetMetric} ${row.targetDir}`
          : statusLabel(row.status)}
      </div>
    </div>
  );
}
