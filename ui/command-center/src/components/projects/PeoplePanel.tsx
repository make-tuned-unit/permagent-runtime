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

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import type { NamedPersonMeeting, Person, Project, ProjectPerson } from './types';

export function PeoplePanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const peopleRev = useCommandCenter(s => s.peopleRev);
  const [people, setPeople] = useState<ProjectPerson[]>([]);
  const [meetings, setMeetings] = useState<NamedPersonMeeting[]>([]);
  const [picking, setPicking] = useState(false);
  // Distinguish first-load, load-failure, and genuinely-empty so a failed fetch
  // never reads the same as "no people associated yet" (and never hangs on a
  // perpetual spinner).
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  // Associate outcome, surfaced inline. Previously the POST failure was swallowed
  // by an empty catch, so a fast 400 was indistinguishable from "the click did
  // nothing" (#561) — the wire is correct, but the failure was invisible.
  const [associateError, setAssociateError] = useState<string | null>(null);
  const loadGeneration = useRef(0);

  // Resolves `false` when the load failed (or was superseded) so the retry
  // button can only tick over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    try {
      const [rows, meetingRows] = await Promise.all([
        apiFetch<ProjectPerson[]>(`/api/projects/${encodeURIComponent(project.id)}/people`),
        apiFetch<NamedPersonMeeting[]>(`/api/projects/${encodeURIComponent(project.id)}/meetings`).catch(() => []),
      ]);
      if (generation !== loadGeneration.current) return false;
      if (!Array.isArray(rows)) throw new Error('Invalid people response');
      setPeople(rows);
      setMeetings(Array.isArray(meetingRows) ? meetingRows : []);
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      // Routes are first-dogfooded here (#530 had no route-level tests); surface
      // the failure as a recoverable error rather than a blank/empty panel.
      setStatus('error');
      return false;
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
      return true;
    } catch (e) {
      // Keep the picker open and show why, so a non-2xx (e.g. a 400 FK reject) is
      // visible instead of looking like a dead click (#561).
      const err = e as Error & { status?: number };
      const status = err.status ? `${err.status} ` : '';
      setAssociateError(`Couldn't associate ${person.display_name}: ${status}${err.message || 'request failed'}`);
      return false;
    }
  };

  return (
    <Panel
      title="People"
      action={
        <Button
          colors={colors}
          variant="bare"
          className="hover:underline"
          onClick={() => { setAssociateError(null); setPicking(v => !v); }}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 'inherit',
            fontFamily: font.body,
            fontSize: 11,
          } as CSSProperties}
        >
          {picking ? 'Close' : '+ Associate'}
        </Button>
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
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={load}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              fontFamily: font.body,
              fontSize: 11,
            } as CSSProperties}
          >
            Retry
          </Button>
        </div>
      ) : people.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>No people associated yet.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {people.map(p => (
            <Button
              key={p.entity_uuid}
              colors={colors}
              variant="ghost"
              onClick={() =>
                openPersonDetail(project.id, p, {
                  project_role: p.project_role,
                  associated_at: p.associated_at,
                })
              }
              // `contents` dissolves Button's `.pa-btn__label` wrapper so the
              // name and the truncating role stay the row's own flex children.
                            style={{
                '--pa-btn-bg': rowVeil,
                '--pa-btn-fg': colors.text,
                '--pa-btn-border': colors.border,
                '--pa-btn-bg-hover': rowVeil,
                '--pa-btn-border-hover': colors.borderHi,
                '--pa-btn-pad': '6px 9px',
                '--pa-btn-radius': '7px',
                '--pa-btn-weight': 'inherit',
                alignItems: 'baseline',
                justifyContent: 'flex-start',
                gap: 8,
                textAlign: 'left',
                width: '100%',
                fontFamily: font.body,
              } as CSSProperties}
            >
              <span style={{ fontSize: 12, color: colors.text, flexShrink: 0 }}>{p.display_name}</span>
              <span style={{
                fontSize: 11, color: colors.textDim, minWidth: 0,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {p.project_role || p.role || ''}
              </span>
            </Button>
          ))}
        </div>
      )}
      {meetings.length > 0 && (
        <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 4 }}>
          <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, textTransform: 'uppercase', letterSpacing: '.04em' }}>
            Meetings
          </div>
          {meetings.slice(0, 8).map(m => (
            <div key={m.id} style={{ fontSize: 11, color: colors.text, padding: '4px 0', borderBottom: `1px solid ${colors.border}` }}>
              <span style={{ fontWeight: 600 }}>{m.display_name}</span>
              <span style={{ color: colors.textDim }}> · {m.title}</span>
            </div>
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
  // `unknown`, not `void`: the caller's promise has to reach the row Button so
  // the round trip drives its pending/success states.
  onPick: (person: Person) => unknown;
}) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Person[]>([]);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');

  useEffect(() => {
    let live = true;
    setStatus('loading');
    const q = query.trim();
    apiFetch<Person[]>(`/api/people${q ? `?q=${encodeURIComponent(q)}` : ''}`)
      .then(rows => {
        if (!Array.isArray(rows)) throw new Error('Invalid people response');
        if (live) { setResults(rows); setStatus('ready'); }
      })
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
            <Button
              key={p.entity_uuid}
              colors={colors}
              variant="bare"
              onClick={() => onPick(p)}
              // `contents` dissolves Button's `.pa-btn__label` wrapper so the
              // name and the truncating subtitle stay the row's flex children.
                            style={{
                '--pa-btn-fg': colors.text,
                '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-pad': '5px 8px',
                '--pa-btn-radius': `${radius.sm}px`,
                '--pa-btn-weight': 'inherit',
                alignItems: 'baseline',
                justifyContent: 'flex-start',
                gap: 8,
                textAlign: 'left',
                width: '100%',
                fontFamily: font.body,
              } as CSSProperties}
            >
              <span style={{ fontSize: 12, flexShrink: 0 }}>{p.display_name}</span>
              <span style={{
                fontSize: 11, color: colors.textDim, minWidth: 0,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {[p.role, p.company].filter(Boolean).join(' · ')}
              </span>
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
