/**
 * StackPanel — the per-project stack organizer (#512) on a project's Overview.
 *
 * Answers "which account do I use for Railway on THIS project?" at a glance:
 * the services the project is built on, grouped by category, each with the
 * login identity (email/handle) used for it, free-text notes (free-tier
 * reality), and an optional dashboard link that opens in the in-app browser
 * (the #506 useBrowserNavigate seam).
 *
 * REFERENCE-ONLY, BY DESIGN — and the copy says so: no passwords or secrets
 * are entered or stored, ever. The identity is a label; the credential stays
 * in the user's password manager. Autofill was researched and ruled out
 * (WKWebView / Associated-Domains limitation), so don't ask for it here.
 *
 * Round-trips through the real mounted endpoints
 * (GET/POST /api/projects/{id}/stack, PATCH/DELETE .../stack/{entry_id}) via
 * stackEntries.ts. Failures surface inline (#568 no-silent-catch rule).
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiEdit2, FiExternalLink, FiPlus, FiTrash2, FiX } from 'react-icons/fi';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import { normalizeUrl } from './workspaceMeta';
import {
  createStackEntry,
  deleteStackEntry,
  groupByCategory,
  listStackEntries,
  updateStackEntry,
} from './stackEntries';
import {
  STACK_CATEGORIES,
  STACK_CATEGORY_LABELS,
  type Project,
  type StackCategory,
  type StackEntry,
} from './types';

/** Composer state — used for both "add" and "edit" (id set = editing). */
interface EntryForm {
  id: string | null;
  serviceName: string;
  category: StackCategory;
  identity: string;
  notes: string;
  dashboardUrl: string;
}

const EMPTY_FORM: EntryForm = {
  id: null,
  serviceName: '',
  category: 'hosting',
  identity: '',
  notes: '',
  dashboardUrl: '',
};

export function StackPanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const navigate = useBrowserNavigate();

  const [entries, setEntries] = useState<StackEntry[]>([]);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  const [form, setForm] = useState<EntryForm | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const loadGeneration = useRef(0);

  // Resolves `false` when the load failed (or was superseded) so the retry
  // button can only tick over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    try {
      const nextEntries = await listStackEntries(project.id);
      if (generation !== loadGeneration.current) return false;
      if (!Array.isArray(nextEntries)) throw new Error('Invalid stack response');
      setEntries(nextEntries);
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      setStatus('error');
      return false;
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  // Resolves `false` on failure: the error is surfaced inline, so the save
  // button must not tick over an entry that never persisted.
  const save = useCallback(async () => {
    if (!form || saving) return false;
    const serviceName = form.serviceName.trim();
    if (!serviceName) return false;
    setError(null);
    setSaving(true);
    try {
      const identity = form.identity.trim();
      const notes = form.notes.trim();
      const dashboardUrl = normalizeUrl(form.dashboardUrl);
      if (form.id) {
        // Edit: send every form field; explicit null clears the emptied ones.
        const updated = await updateStackEntry(project.id, form.id, {
          serviceName,
          category: form.category,
          identity: identity || null,
          notes,
          dashboardUrl: dashboardUrl ?? null,
        });
        setEntries(prev => prev.map(e => (e.id === updated.id ? updated : e)));
      } else {
        const created = await createStackEntry(project.id, {
          serviceName,
          category: form.category,
          identity: identity || undefined,
          notes: notes || undefined,
          dashboardUrl: dashboardUrl ?? undefined,
        });
        setEntries(prev => [...prev, created]);
      }
      setForm(null);
      return true;
    } catch (e) {
      setError(`Couldn't save entry: ${(e as Error).message || 'request failed'}`);
      return false;
    } finally {
      setSaving(false);
    }
  }, [project.id, form, saving]);

  const remove = async (entry: StackEntry) => {
    setError(null);
    try {
      await deleteStackEntry(project.id, entry.id);
      setEntries(prev => prev.filter(e => e.id !== entry.id));
      return true;
    } catch (e) {
      setError(`Couldn't remove entry: ${(e as Error).message || 'request failed'}`);
      return false;
    }
  };

  const startEdit = (entry: StackEntry) => {
    setError(null);
    setForm({
      id: entry.id,
      serviceName: entry.service_name,
      category: entry.category,
      identity: entry.identity ?? '',
      notes: entry.notes,
      dashboardUrl: entry.dashboard_url ?? '',
    });
  };

  const inputStyle: React.CSSProperties = {
    fontSize: 12, padding: '6px 9px', borderRadius: 7,
    background: colors.inputBg, border: `1px solid ${colors.border}`,
    color: colors.text, fontFamily: font.body, outline: 'none',
  };

  const groups = groupByCategory(entries);

  return (
    <Panel
      title="Stack"
      action={
        form ? (
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={() => { setForm(null); setError(null); }}
            style={{
              '--pa-btn-fg': colors.textDim,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              gap: 4,
              fontFamily: font.body,
              fontSize: 11,
            } as CSSProperties}
          >
            <FiX size={11} /> Cancel
          </Button>
        ) : (
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={() => { setError(null); setForm(EMPTY_FORM); }}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              gap: 4,
              fontFamily: font.body,
              fontSize: 11,
            } as CSSProperties}
          >
            <FiPlus size={11} /> Add service
          </Button>
        )
      }
    >
      {/* Composer (add or edit) */}
      {form && (
        <div style={{
          display: 'flex', flexDirection: 'column', gap: 6, marginBottom: 10,
          padding: '10px', borderRadius: 7, background: rowVeil,
          border: `1px solid ${colors.borderHi}`,
        }}>
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              value={form.serviceName}
              onChange={e => setForm({ ...form, serviceName: e.target.value })}
              placeholder="Service — e.g. Vercel, Neon, Railway"
              autoFocus
              style={{ ...inputStyle, flex: 1, minWidth: 0 }}
            />
            <select
              value={form.category}
              onChange={e => setForm({ ...form, category: e.target.value as StackCategory })}
              style={{ ...inputStyle, width: 110, flexShrink: 0 }}
            >
              {STACK_CATEGORIES.map(c => (
                <option key={c} value={c}>{STACK_CATEGORY_LABELS[c]}</option>
              ))}
            </select>
          </div>
          <input
            value={form.identity}
            onChange={e => setForm({ ...form, identity: e.target.value })}
            placeholder="Login identity — the email/handle you sign in with (never a password)"
            style={inputStyle}
          />
          <input
            value={form.dashboardUrl}
            onChange={e => setForm({ ...form, dashboardUrl: e.target.value })}
            placeholder="Dashboard URL (optional)"
            style={inputStyle}
          />
          <input
            value={form.notes}
            onChange={e => setForm({ ...form, notes: e.target.value })}
            onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); save(); } }}
            placeholder='Notes — e.g. "free tier, 2 projects max"'
            style={inputStyle}
          />
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Button
              colors={colors}
              variant="ghostOn"
              onClick={save}
              disabled={!form.serviceName.trim() || saving}
              style={{
                '--pa-btn-bg': colors.cyanSoft,
                '--pa-btn-border': colors.borderHi,
                '--pa-btn-pad': '6px 14px',
                '--pa-btn-radius': '7px',
                '--pa-btn-weight': 600,
                fontFamily: font.body,
                fontSize: 12,
              } as CSSProperties}
            >
              {saving ? 'Saving…' : form.id ? 'Save changes' : 'Add to stack'}
            </Button>
            <span style={{ fontSize: 10, color: colors.textDim }}>
              No passwords or secrets — those stay in your password manager.
            </span>
          </div>
        </div>
      )}

      {error && (
        <div style={{ fontSize: 11, color: colors.danger, marginBottom: 8 }}>{error}</div>
      )}

      {status === 'loading' && (
        <div style={{ fontSize: 11, color: colors.textDim }}>Loading stack…</div>
      )}

      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: 11, color: colors.danger }}>Couldn't load the stack.</span>
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
      )}

      {status === 'ready' && entries.length === 0 && !form && (
        <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
          No services yet — add the ones this project runs on and which account
          you use for each. Reference only: no passwords or secrets, ever.
        </div>
      )}

      {status === 'ready' && groups.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {groups.map(group => (
            <div key={group.category}>
              <div style={{
                fontSize: 10, fontWeight: 600, color: colors.textDim,
                textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 4,
              }}>
                {STACK_CATEGORY_LABELS[group.category]}
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {group.entries.map(entry => (
                  <div
                    key={entry.id}
                    style={{
                      display: 'flex', alignItems: 'flex-start', gap: 8, padding: '8px 10px',
                      borderRadius: 7, background: rowVeil, border: `1px solid ${colors.border}`,
                      transition: 'border-color 150ms',
                    }}
                    onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
                    onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
                  >
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
                        <span style={{ fontSize: 12, fontWeight: 600, color: colors.text }}>
                          {entry.service_name}
                        </span>
                        {entry.identity ? (
                          <span
                            title="The login identity used for this service on this project"
                            style={{
                              fontSize: 11, color: colors.cyan, fontFamily: font.mono,
                              overflowWrap: 'anywhere',
                            }}
                          >
                            {entry.identity}
                          </span>
                        ) : (
                          <span style={{ fontSize: 11, color: colors.textDim, fontStyle: 'italic' }}>
                            no identity recorded
                          </span>
                        )}
                      </div>
                      {entry.notes && (
                        <div style={{
                          fontSize: 11, color: colors.textMuted, lineHeight: 1.5,
                          marginTop: 2, overflowWrap: 'anywhere',
                        }}>
                          {entry.notes}
                        </div>
                      )}
                      {entry.dashboard_url && (
                        <Button
                          colors={colors}
                          variant="bare"
                          className="hover:underline"
                          onClick={() => navigate(entry.dashboard_url!)}
                          title={`Open ${entry.dashboard_url} in the in-app browser`}
                          style={{
                            '--pa-btn-fg': colors.cyan,
                            '--pa-btn-bg-hover': 'transparent',
                            '--pa-btn-pad': '0',
                            '--pa-btn-weight': 600,
                            marginTop: 4,
                            gap: 4,
                            fontFamily: font.body,
                            fontSize: 10,
                          } as CSSProperties}
                        >
                          Dashboard <FiExternalLink size={9} />
                        </Button>
                      )}
                    </div>
                    <Button
                      colors={colors}
                      variant="bare"
                      onClick={() => startEdit(entry)}
                      title="Edit entry"
                      aria-label="Edit entry"
                      style={{
                        '--pa-btn-fg': colors.textDim,
                        '--pa-btn-fg-hover': colors.cyan,
                        '--pa-btn-bg-hover': 'transparent',
                        '--pa-btn-pad': '2px',
                        flexShrink: 0,
                      } as CSSProperties}
                    >
                      <FiEdit2 size={13} />
                    </Button>
                    <Button
                      colors={colors}
                      variant="bare"
                      onClick={() => remove(entry)}
                      title="Remove entry"
                      aria-label="Remove entry"
                      style={{
                        '--pa-btn-fg': colors.textDim,
                        '--pa-btn-fg-hover': colors.danger,
                        '--pa-btn-bg-hover': 'transparent',
                        '--pa-btn-pad': '2px',
                        flexShrink: 0,
                      } as CSSProperties}
                    >
                      <FiTrash2 size={13} />
                    </Button>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}
