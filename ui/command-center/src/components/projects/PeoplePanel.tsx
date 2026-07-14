/**
 * PeoplePanel (CRM epic slice 2a) — the People panel on a project's Overview.
 *
 * Renders the people associated with the project (#530
 * `GET /api/projects/{id}/people`); clicking a row opens the read-only
 * PersonDetailModal via the store. The "+ Associate" action reveals an inline
 * picker that searches the global CRM directory (`GET /api/people?q=`) and
 * associates an existing person (`POST /api/projects/{id}/people`,
 * camelCase body `{ entityUuid, role? }`). Associate-from-existing only —
 * creating a new person is out of scope for this slice.
 *
 * The panel re-fetches on `peopleRev`, the store signal the person modal bumps
 * after a disassociate (there is no people event stream yet).
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Panel } from './Panel';
import type { Person, Project, ProjectPerson } from './types';

export function PeoplePanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const peopleRev = useCommandCenter(s => s.peopleRev);
  const [people, setPeople] = useState<ProjectPerson[]>([]);
  const [picking, setPicking] = useState(false);
  // Distinguish first-load, load-failure, and genuinely-empty so a failed fetch
  // never reads the same as "no people associated yet" (and never hangs on a
  // perpetual spinner).
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  // Associate outcome, surfaced inline. Previously the POST failure was swallowed
  // by an empty catch, so a fast 400 was indistinguishable from "the click did
  // nothing" (#561) — the wire is correct, but the failure was invisible.
  const [associateError, setAssociateError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const rows = await apiFetch<ProjectPerson[]>(`/api/projects/${encodeURIComponent(project.id)}/people`);
      setPeople(rows);
      setStatus('ready');
    } catch {
      // Routes are first-dogfooded here (#530 had no route-level tests); surface
      // the failure as a recoverable error rather than a blank/empty panel.
      setStatus('error');
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load, peopleRev]);

  const associatedIds = useMemo(() => new Set(people.map(p => p.entity_uuid)), [people]);

  const associate = async (person: Person) => {
    setAssociateError(null);
    try {
      await apiFetch(`/api/projects/${encodeURIComponent(project.id)}/people`, {
        method: 'POST',
        body: JSON.stringify({ entityUuid: person.entity_uuid }),
      });
      // Success: close the picker and re-fetch — the person moves into the list.
      setPicking(false);
      bumpPeople();
    } catch (e) {
      // Keep the picker open and show why, so a non-2xx (e.g. a 400 FK reject) is
      // visible instead of looking like a dead click (#561).
      const err = e as Error & { status?: number };
      const status = err.status ? `${err.status} ` : '';
      setAssociateError(`Couldn't associate ${person.display_name}: ${status}${err.message || 'request failed'}`);
    }
  };

  return (
    <Panel
      title="People"
      action={
        <button
          onClick={() => { setAssociateError(null); setPicking(v => !v); }}
          style={{
            fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
            cursor: 'pointer', fontFamily: font.body, padding: 0,
          }}
        >
          {picking ? 'Close' : '+ Associate'}
        </button>
      }
    >
      {picking && (
        <AssociatePicker
          colors={colors}
          excludeIds={associatedIds}
          onPick={associate}
        />
      )}

      {associateError && (
        <div style={{ fontSize: 11, color: colors.danger, marginBottom: 8 }}>
          {associateError}
        </div>
      )}

      {status === 'loading' ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>Loading people…</div>
      ) : status === 'error' ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: 11, color: colors.danger }}>Couldn't load people.</span>
          <button
            onClick={load}
            style={{
              fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
              cursor: 'pointer', fontFamily: font.body, padding: 0, fontWeight: 600,
            }}
          >
            Retry
          </button>
        </div>
      ) : people.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>No people associated yet.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {people.map(p => (
            <button
              key={p.entity_uuid}
              onClick={() => openPersonDetail(project.id, p)}
              style={{
                display: 'flex', alignItems: 'baseline', gap: 8, textAlign: 'left',
                padding: '6px 9px', borderRadius: 7, width: '100%',
                background: rowVeil, border: `1px solid ${colors.border}`,
                color: colors.text, fontFamily: font.body, cursor: 'pointer', transition: 'border-color 150ms',
              }}
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
            >
              <span style={{ fontSize: 12, color: colors.text, flexShrink: 0 }}>{p.display_name}</span>
              <span style={{
                fontSize: 11, color: colors.textDim, minWidth: 0,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {p.project_role || p.role || ''}
              </span>
            </button>
          ))}
        </div>
      )}
    </Panel>
  );
}

// ── Associate picker ─────────────────────────────────────────────────────────

function AssociatePicker({ colors, excludeIds, onPick }: {
  colors: ReturnType<typeof useTheme>['colors'];
  excludeIds: Set<string>;
  onPick: (person: Person) => void;
}) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Person[]>([]);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');

  useEffect(() => {
    let live = true;
    setStatus('loading');
    const q = query.trim();
    apiFetch<Person[]>(`/api/people${q ? `?q=${encodeURIComponent(q)}` : ''}`)
      .then(rows => { if (live) { setResults(rows); setStatus('ready'); } })
      .catch(() => { if (live) { setResults([]); setStatus('error'); } });
    return () => { live = false; };
  }, [query]);

  const candidates = results.filter(p => !excludeIds.has(p.entity_uuid));

  return (
    <div style={{ marginBottom: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
      <input
        autoFocus
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder="Search people…"
        style={{
          fontSize: 12, padding: '6px 9px', borderRadius: 7,
          background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
          color: colors.text, fontFamily: font.body, outline: 'none',
        }}
      />
      {status === 'loading' ? (
        <div style={{ fontSize: 11, color: colors.textDim, padding: '2px 2px' }}>Searching…</div>
      ) : status === 'error' ? (
        <div style={{ fontSize: 11, color: colors.danger, padding: '2px 2px' }}>Couldn't search the directory.</div>
      ) : candidates.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim, padding: '2px 2px' }}>
          {results.length === 0 ? 'No people in the directory.' : 'No more to add.'}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, maxHeight: 240, overflowY: 'auto' }}>
          {candidates.map(p => (
            <button
              key={p.entity_uuid}
              onClick={() => onPick(p)}
              style={{
                display: 'flex', alignItems: 'baseline', gap: 8, textAlign: 'left',
                padding: '5px 8px', borderRadius: 6, width: '100%',
                background: 'none', border: 'none', color: colors.text,
                fontFamily: font.body, cursor: 'pointer',
              }}
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = colors.cyanSoft; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'none'; }}
            >
              <span style={{ fontSize: 12, flexShrink: 0 }}>{p.display_name}</span>
              <span style={{
                fontSize: 11, color: colors.textDim, minWidth: 0,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {[p.role, p.company].filter(Boolean).join(' · ')}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
