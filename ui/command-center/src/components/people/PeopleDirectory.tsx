/**
 * PeopleDirectory — the global people surface.
 *
 * Until this existed there was no way to see everyone. People lived in exactly
 * two places and nothing aggregated them: `PeoplePanel`, which renders only
 * inside one project's Overview and lists only `project_people` rows, and the
 * Brain's entity graph. On the live corpus that left **15 of 24 people with no
 * project association at all** — reachable from no UI whatsoever. That integer
 * is what this component exists to move, and it is readable:
 *
 *   sqlite3 ~/.permagent/spectral/permagent.db "select count(*) from people p \
 *     where not exists (select 1 from project_people pp \
 *                       where pp.entity_uuid = p.entity_uuid)"
 *
 * Reads `GET /api/people/directory` (every person + their project chips, two
 * queries server-side). The search box is **debounced**: unlike the associate
 * picker's live search over `GET /api/people`, this endpoint also runs the graph
 * attribute overlay over every row and logs a latency line per call, so a
 * per-keystroke fetch would flood both the brain read and the daemon log.
 *
 * Re-fetches on `peopleRev`, which `livenessSync` bumps on `person_changed` —
 * so a person created on another client (or the phone) lands here.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { contactLabel, isFollowUpDue, isQuiet } from './contactAge';

type Status = 'loading' | 'error' | 'ready';

/**
 * Flag likely duplicates without acting on them.
 *
 * Prefix-match only, and deliberately so. Two richer rules were tried against
 * the real 24-row corpus and dropped: matching on shared surname flags
 * "Leanne Dixon" / "Liam Dixon" (two different people, one family), and
 * matching on email domain can never fire at all — the graph overlay clears the
 * people-table columns and refills only from `entity_fields`, so `email` is null
 * on the wire for every person nobody has manually edited.
 *
 * There is no merge primitive anywhere in the codebase, so this is a *label*,
 * never an action. Its job is to make a false split visible; a directory is the
 * first surface where duplicates become obvious and it must not look broken
 * when they appear.
 */
function duplicateIds(people: DirectoryPerson[]): Set<string> {
  const norm = (s: string) => s.trim().toLowerCase().replace(/\s+/g, ' ');
  const flagged = new Set<string>();
  for (let i = 0; i < people.length; i++) {
    for (let j = i + 1; j < people.length; j++) {
      const a = norm(people[i].display_name);
      const b = norm(people[j].display_name);
      if (!a || !b || a === b) continue;
      const [short, long] = a.length < b.length ? [a, b] : [b, a];
      if (long.startsWith(`${short} `)) {
        flagged.add(people[i].entity_uuid);
        flagged.add(people[j].entity_uuid);
      }
    }
  }
  return flagged;
}

export function PeopleDirectory() {
  const { colors, theme } = useTheme();
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const peopleRev = useCommandCenter(s => s.peopleRev);

  const [people, setPeople] = useState<DirectoryPerson[]>([]);
  const [status, setStatus] = useState<Status>('loading');
  const [query, setQuery] = useState('');
  const [debouncedQuery, setDebouncedQuery] = useState('');
  const [adding, setAdding] = useState(false);
  const [newName, setNewName] = useState('');
  const [addError, setAddError] = useState<string | null>(null);
  const [addBusy, setAddBusy] = useState(false);
  const [cohort, setCohort] = useState<'all' | 'quiet' | 'followup'>('all');
  /**
   * True when rows came back but every attribute is null — the shape of a
   * Brain-down daemon, because the overlay clears the columns and then returns
   * early. Without this the directory renders a list of bare names that looks
   * exactly like data loss.
   */
  const [attributesBlank, setAttributesBlank] = useState(false);
  const loadGeneration = useRef(0);

  useEffect(() => {
    const t = setTimeout(() => setDebouncedQuery(query), 200);
    return () => clearTimeout(t);
  }, [query]);

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    try {
      const suffix = debouncedQuery.trim()
        ? `?q=${encodeURIComponent(debouncedQuery.trim())}`
        : '';
      const rows = await apiFetch<DirectoryPerson[]>(`/api/people/directory${suffix}`);
      if (generation !== loadGeneration.current) return;
      if (!Array.isArray(rows)) throw new Error('Invalid directory response');
      setPeople(rows);
      setAttributesBlank(
        rows.length > 0 &&
          rows.every(p => !p.role && !p.company && !p.email && !p.phone),
      );
      setStatus('ready');
    } catch {
      if (generation !== loadGeneration.current) return;
      setStatus('error');
    }
  }, [debouncedQuery]);

  useEffect(() => {
    load();
  }, [load, peopleRev]);

  const duplicates = useMemo(() => duplicateIds(people), [people]);
  const unassignedCount = useMemo(
    () => people.filter(p => p.projects.length === 0).length,
    [people],
  );
  const quietCount = useMemo(() => people.filter(p => isQuiet(p.last_contact_at)).length, [people]);
  const followUpCount = useMemo(
    () => people.filter(p => isFollowUpDue(p.next_follow_up_at)).length,
    [people],
  );
  const visible = useMemo(() => {
    const filtered = people.filter(p => {
      if (cohort === 'quiet') return isQuiet(p.last_contact_at);
      if (cohort === 'followup') return isFollowUpDue(p.next_follow_up_at);
      return true;
    });
    return [...filtered].sort((a, b) => {
      const aDue = isFollowUpDue(a.next_follow_up_at) ? 0 : 1;
      const bDue = isFollowUpDue(b.next_follow_up_at) ? 0 : 1;
      if (aDue !== bDue) return aDue - bDue;
      const aQ = isQuiet(a.last_contact_at) ? 0 : 1;
      const bQ = isQuiet(b.last_contact_at) ? 0 : 1;
      if (aQ !== bQ) return aQ - bQ;
      return a.display_name.localeCompare(b.display_name);
    });
  }, [people, cohort]);

  const addPerson = async () => {
    const name = newName.trim();
    if (!name) return;
    setAddBusy(true);
    setAddError(null);
    try {
      const res = await apiFetch<{ person: DirectoryPerson; created: boolean }>(
        '/api/people',
        {
          // snake_case: the People endpoints carry no serde rename_all.
          method: 'POST',
          body: JSON.stringify({ display_name: name }),
        },
      );
      setAdding(false);
      setNewName('');
      bumpPeople();
      if (!res.created) {
        // Not an error, but never a silent success either — the caller asked for
        // a new person and got an existing one.
        setAddError(`"${res.person.display_name}" already exists — opening them.`);
      }
      openPersonDetail(null, { ...res.person });
    } catch (e) {
      const err = e as Error & { status?: number };
      const code = err.status ? `${err.status} ` : '';
      setAddError(`Couldn't add ${name}: ${code}${err.message || 'request failed'}`);
    } finally {
      setAddBusy(false);
    }
  };

  return (
    <div style={{ padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <input
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="Search people…"
          style={{
            flex: 1,
            maxWidth: 320,
            fontSize: 12,
            fontFamily: font.body,
            padding: '6px 10px',
            borderRadius: 6,
            border: `1px solid ${colors.border}`,
            background: 'transparent',
            color: colors.text,
            outline: 'none',
          }}
        />
        <span style={{ flex: 1 }} />
        <button
          onClick={() => {
            setAddError(null);
            setAdding(v => !v);
          }}
          style={{
            fontSize: 11,
            color: colors.cyan,
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            fontFamily: font.body,
            padding: 0,
          }}
        >
          {adding ? 'Cancel' : '+ Add person'}
        </button>
      </div>

      {adding && (
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <input
            autoFocus
            value={newName}
            onChange={e => setNewName(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') addPerson();
            }}
            placeholder="Full name"
            style={{
              fontSize: 12,
              fontFamily: font.body,
              padding: '6px 10px',
              borderRadius: 6,
              border: `1px solid ${colors.border}`,
              background: 'transparent',
              color: colors.text,
              outline: 'none',
            }}
          />
          <button
            onClick={addPerson}
            disabled={addBusy || !newName.trim()}
            style={{
              fontSize: 11,
              color: colors.cyan,
              background: 'none',
              border: 'none',
              cursor: addBusy || !newName.trim() ? 'default' : 'pointer',
              opacity: addBusy || !newName.trim() ? 0.5 : 1,
              fontFamily: font.body,
              padding: 0,
              fontWeight: 600,
            }}
          >
            {addBusy ? 'Adding…' : 'Add'}
          </button>
        </div>
      )}

      {addError && <div style={{ fontSize: 11, color: colors.textDim }}>{addError}</div>}

      {attributesBlank && status === 'ready' && (
        <div style={{ fontSize: 11, color: colors.textDim }}>
          Showing names only — the Brain isn't available, so roles, companies and
          contact details can't be read right now.
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
              fontSize: 11,
              color: colors.cyan,
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              fontFamily: font.body,
              padding: 0,
              fontWeight: 600,
            }}
          >
            Retry
          </button>
        </div>
      ) : people.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>
          {debouncedQuery.trim() ? 'No people match that search.' : 'No people yet.'}
        </div>
      ) : (
        <>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono }}>
              {visible.length} {visible.length === 1 ? 'person' : 'people'}
              {unassignedCount > 0 && ` · ${unassignedCount} in no project`}
            </div>
            <span style={{ flex: 1 }} />
            {([
              ['all', 'All'],
              ['quiet', `Quiet${quietCount ? ` ${quietCount}` : ''}`],
              ['followup', `Follow-up${followUpCount ? ` ${followUpCount}` : ''}`],
            ] as const).map(([key, label]) => (
              <button
                key={key}
                onClick={() => setCohort(key)}
                style={{
                  fontSize: 11,
                  fontFamily: font.body,
                  padding: '2px 8px',
                  borderRadius: 4,
                  cursor: 'pointer',
                  border: `1px solid ${cohort === key ? colors.cyan : colors.border}`,
                  background: cohort === key ? colors.cyanSoft : 'transparent',
                  color: cohort === key ? colors.cyan : colors.textMuted,
                }}
              >
                {label}
              </button>
            ))}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {visible.map(p => (
              <button
                key={p.entity_uuid}
                onClick={() => openPersonDetail(null, p)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  textAlign: 'left',
                  background: rowVeil,
                  border: `1px solid ${colors.border}`,
                  borderRadius: 6,
                  padding: '8px 10px',
                  cursor: 'pointer',
                  fontFamily: font.body,
                  color: colors.text,
                }}
              >
                <span style={{ fontSize: 12, fontWeight: 600 }}>{p.display_name}</span>
                {(p.role || p.company) && (
                  <span style={{ fontSize: 11, color: colors.textDim }}>
                    {[p.role, p.company].filter(Boolean).join(' · ')}
                  </span>
                )}
                {duplicates.has(p.entity_uuid) && (
                  <span
                    title="Another person has a very similar name. Nothing has been merged."
                    style={{
                      fontSize: 10,
                      color: colors.textDim,
                      border: `1px solid ${colors.border}`,
                      borderRadius: 4,
                      padding: '1px 5px',
                    }}
                  >
                    possible duplicate
                  </span>
                )}
                {isFollowUpDue(p.next_follow_up_at) && (
                  <span style={{ fontSize: 10, color: colors.cyan, border: `1px solid ${colors.cyan}`, borderRadius: 4, padding: '1px 5px' }}>
                    follow up
                  </span>
                )}
                <span style={{ flex: 1 }} />
                <span style={{ fontSize: 10, color: isQuiet(p.last_contact_at) ? colors.textDim : colors.textMuted, fontFamily: font.mono }}>
                  {contactLabel(p.last_contact_at)}
                </span>
                {p.projects.length === 0 ? (
                  <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>
                    no project
                  </span>
                ) : (
                  <span style={{ display: 'flex', gap: 4 }}>
                    {p.projects.map(pr => (
                      <span
                        key={pr.project_id}
                        style={{
                          fontSize: 10,
                          color: colors.cyan,
                          background: colors.cyanSoft,
                          borderRadius: 4,
                          padding: '1px 6px',
                        }}
                      >
                        {pr.project_name}
                      </span>
                    ))}
                  </span>
                )}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
