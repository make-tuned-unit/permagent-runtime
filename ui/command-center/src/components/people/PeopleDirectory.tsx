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

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { contactLabel, isFollowUpDue, isQuiet } from './contactAge';
import type { DirectoryPerson } from '../projects/types';
import { Button } from '../common/Button';
import { Chip } from '../common/Chip';

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
 * This is a *label*, never an action — but not for the reason this comment used
 * to give. It claimed no merge primitive existed anywhere in the codebase, and
 * that stopped being true: `MergePersonPanel` implements a full three-step
 * merge with an undo, reachable from the person detail modal. The real reason
 * is placement. Merging is a decision made while looking at both people, which
 * is what the detail modal shows and a directory row does not.
 *
 * Its job here is to make a false split visible; a directory is the first
 * surface where duplicates become obvious and it must not look broken when
 * they appear.
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
  /** The surface's three text affordances (+ Add person / Add / Retry). They
   *  are ink on nothing, so hover brightens the ink rather than painting a box
   *  the label never had. */
  const linkVars = (): CSSProperties => ({
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-fg-hover': colors.cyan,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
    '--pa-btn-pad': '0',
    '--pa-btn-weight': 400,
    fontFamily: font.body,
    fontSize: textSize.micro,
  } as CSSProperties);
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

  /** Resolves whether this load landed. The Retry button awaits it, and this
   *  swallows its failure into `status`, so it has to say so. */
  const load = useCallback(async (): Promise<boolean> => {
    const generation = ++loadGeneration.current;
    try {
      const suffix = debouncedQuery.trim()
        ? `?q=${encodeURIComponent(debouncedQuery.trim())}`
        : '';
      const rows = await apiFetch<DirectoryPerson[]>(`/api/people/directory${suffix}`);
      if (generation !== loadGeneration.current) return false;
      if (!Array.isArray(rows)) throw new Error('Invalid directory response');
      setPeople(rows);
      setAttributesBlank(
        rows.length > 0 &&
          rows.every(p => !p.role && !p.company && !p.email && !p.phone),
      );
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      setStatus('error');
      return false;
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

  /** Resolves false when nothing was added — the failure is swallowed into
   *  `addError` below, so the button must not tick over the top of it. */
  const addPerson = async (): Promise<boolean> => {
    const name = newName.trim();
    if (!name) return false;
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
      return true;
    } catch (e) {
      const err = e as Error & { status?: number };
      const code = err.status ? `${err.status} ` : '';
      setAddError(`Couldn't add ${name}: ${code}${err.message || 'request failed'}`);
      return false;
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
            fontSize: textSize.caption,
            fontFamily: font.body,
            padding: '6px 10px',
            borderRadius: radius.sm,
            border: `1px solid ${colors.border}`,
            background: 'transparent',
            color: colors.text,
            outline: 'none',
          }}
        />
        <span style={{ flex: 1 }} />
        <Button
          colors={colors}
          variant="bare"
          onClick={() => {
            setAddError(null);
            setAdding(v => !v);
          }}
          className="hover:underline"
          style={linkVars()}
        >
          {adding ? 'Cancel' : '+ Add person'}
        </Button>
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
              fontSize: textSize.caption,
              fontFamily: font.body,
              padding: '6px 10px',
              borderRadius: radius.sm,
              border: `1px solid ${colors.border}`,
              background: 'transparent',
              color: colors.text,
              outline: 'none',
            }}
          />
          <Button
            colors={colors}
            variant="bare"
            onClick={addPerson}
            disabled={addBusy || !newName.trim()}
            className="hover:underline"
            style={{ ...linkVars(), '--pa-btn-weight': 600 } as CSSProperties}
          >
            {addBusy ? 'Adding…' : 'Add'}
          </Button>
        </div>
      )}

      {addError && <div style={{ fontSize: textSize.micro, color: colors.textDim }}>{addError}</div>}

      {attributesBlank && status === 'ready' && (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
          Showing names only — the Brain isn't available, so roles, companies and
          contact details can't be read right now.
        </div>
      )}

      {status === 'loading' ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>Loading people…</div>
      ) : status === 'error' ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: textSize.micro, color: colors.danger }}>Couldn't load people.</span>
          <Button
            colors={colors}
            variant="bare"
            onClick={load}
            className="hover:underline"
            style={{ ...linkVars(), '--pa-btn-weight': 600 } as CSSProperties}
          >
            Retry
          </Button>
        </div>
      ) : people.length === 0 ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
          {debouncedQuery.trim() ? 'No people match that search.' : 'No people yet.'}
        </div>
      ) : (
        <>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono }}>
              {visible.length} {visible.length === 1 ? 'person' : 'people'}
              {unassignedCount > 0 && ` · ${unassignedCount} in no project`}
            </div>
            <span style={{ flex: 1 }} />
            {/* Filters, and now typed as filters: `kind="filter"` renders a
                button that reports `aria-pressed`, which these carried no way
                of announcing when they were hand-rolled. */}
            {([
              ['all', 'All'],
              ['quiet', `Quiet${quietCount ? ` ${quietCount}` : ''}`],
              ['followup', `Follow-up${followUpCount ? ` ${followUpCount}` : ''}`],
            ] as const).map(([key, label]) => (
              <Chip
                key={key}
                kind="filter"
                tone="accent"
                pressed={cohort === key}
                onClick={() => setCohort(key)}
                data-testid={`people-cohort-${key}`}
              >
                {label}
              </Chip>
            ))}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {visible.map(p => (
              // The row IS the button, and it distributes its own children
              // against a `flex: 1` spacer — so the primitive's label wrapper
              // is dissolved with `display: contents` and the children stay
              // direct flex children, laid out exactly as before. What is new
              // is that a clickable row finally acknowledges the pointer.
              <Button
                key={p.entity_uuid}
                colors={colors}
                onClick={() => openPersonDetail(null, p)}
                                style={{
                  '--pa-btn-bg': rowVeil,
                  '--pa-btn-fg': colors.text,
                  '--pa-btn-border': colors.border,
                  '--pa-btn-bg-hover': colors.surfaceHi,
                  '--pa-btn-border-hover': colors.borderHi,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-bg-active': colors.surface,
                  '--pa-btn-pad': '8px 10px',
                  '--pa-btn-radius': `${radius.sm}px`,
                  '--pa-btn-weight': 400,
                  gap: 10,
                  textAlign: 'left',
                  fontFamily: font.body,
                } as CSSProperties}
              >
                <span style={{ fontSize: textSize.caption, fontWeight: 600 }}>{p.display_name}</span>
                {(p.role || p.company) && (
                  <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
                    {[p.role, p.company].filter(Boolean).join(' · ')}
                  </span>
                )}
                {/* A flag, not a status: nothing is watching for this to change
                    and it will say the same thing tomorrow. `kind="static"`
                    draws it as the outline it is, so it cannot be mistaken for
                    something live sitting in the same row. */}
                {duplicates.has(p.entity_uuid) && (
                  <Chip
                    kind="static"
                    title="Another person has a very similar name. Nothing has been merged."
                  >
                    possible duplicate
                  </Chip>
                )}
                {isFollowUpDue(p.next_follow_up_at) && (
                  <Chip kind="static" tone="accent" title="A follow-up on this person is due">
                    follow up
                  </Chip>
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
                      // A label on a row that is itself the button — the chip
                      // does not navigate, the row does.
                      <Chip key={pr.project_id} kind="static" tone="accent">
                        {pr.project_name}
                      </Chip>
                    ))}
                  </span>
                )}
              </Button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
