/**
 * PersonSearchPicker — a debounced, inline search over the global CRM
 * directory (`GET /api/people?q=`), for picking ONE existing person.
 *
 * Extracted from `projects/PeoplePanel.tsx`'s local `AssociatePicker` (dagK
 * owner-identity, 2026-09-26; R7 convergence — two surfaces needed the exact
 * same "search people, click one to pick it" interaction: a project's
 * "+ Associate" and Settings → Preferences' "This is me in People"). Rather
 * than let a second copy diverge, this is now the one component both use;
 * `PeoplePanel` re-exports nothing special, it just imports this.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import type { Person } from '../projects/types';

export function PersonSearchPicker({ excludeIds, onPick, placeholder = 'Search people…' }: {
  /** Directory rows already accounted for elsewhere — hidden from results
   *  rather than shown as a redundant pick target. */
  excludeIds?: Set<string>;
  // `unknown`, not `void`: the caller's promise has to reach the row Button so
  // the round trip drives its pending/success states.
  onPick: (person: Person) => unknown;
  placeholder?: string;
}) {
  const { colors } = useTheme();
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

  const candidates = excludeIds ? results.filter(p => !excludeIds.has(p.entity_uuid)) : results;

  return (
    <div style={{ marginBottom: space.lg, display: 'flex', flexDirection: 'column', gap: space.sm }}>
      <input
        autoFocus
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder={placeholder}
        style={{
          fontSize: textSize.caption, padding: `${space.sm}px ${space.md}px`, borderRadius: radius.md,
          background: colors.fillSubtle, border: `1px solid ${colors.border}`,
          color: colors.text, fontFamily: font.body, outline: 'none',
        }}
      />
      {status === 'loading' ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim, padding: `${space.xxs}px ${space.xxs}px` }}>Searching…</div>
      ) : status === 'error' ? (
        <div style={{ fontSize: textSize.micro, color: colors.danger, padding: `${space.xxs}px ${space.xxs}px` }}>Couldn't search the directory.</div>
      ) : candidates.length === 0 ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim, padding: `${space.xxs}px ${space.xxs}px` }}>
          {results.length === 0 ? 'No people in the directory.' : 'No more to add.'}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxs, maxHeight: 240, overflowY: 'auto' }}>
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
                gap: space.md,
                textAlign: 'left',
                width: '100%',
                fontFamily: font.body,
              } as CSSProperties}
            >
              <span style={{ fontSize: textSize.caption, flexShrink: 0 }}>{p.display_name}</span>
              <span style={{
                fontSize: textSize.micro, color: colors.textDim, minWidth: 0,
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
