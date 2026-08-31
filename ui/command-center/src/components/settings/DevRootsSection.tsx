import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { api } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { font, radius } from '../../styles/tokens';
import { Button } from '../common/Button';

/**
 * The permanent home of the "where do you keep your code?" answer.
 *
 * The onboarding step (MomentCode) can be skipped, and the wizard tells the
 * user they can set it later here — so this pane has to exist for that sentence
 * to be true. It is also the only place an existing user, who onboarded before
 * the question was asked, can supply the answer at all.
 *
 * Everything about disk cleanup, project discovery, and the Picker checkout
 * reads `dev_roots`. When it is empty those features fall back to guessing, and
 * a wrong guess produces an empty result that looks exactly like good news.
 */
export function DevRootsSection() {
  const { colors } = useTheme();

  const [roots, setRoots] = useState<string[]>([]);
  const [discovered, setDiscovered] = useState<string[]>([]);
  const [home, setHome] = useState('');
  const [loading, setLoading] = useState(true);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ kind: 'error' | 'warn' | 'ok'; text: string } | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await api.getDevRoots();
      setRoots(r.confirmed);
      setDiscovered(r.discovered.filter(d => !r.confirmed.includes(d)));
      setHome(r.home);
    } catch (e) {
      setNote({ kind: 'error', text: `Couldn't read your settings (${e instanceof Error ? e.message : String(e)}).` });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  // Persist first, then reflect. Showing the new list before the write lands
  // would let a failed save look like a successful one, which is the exact
  // failure mode this whole feature exists to remove.
  // Both of these resolve `false` on every path that did not change anything —
  // the Button contract's "it failed" signal. The catches here swallow their
  // throw into a `note`, so without it a save that was rejected, or a folder
  // that does not exist, would still finish with the button's success tick.
  const persist = async (next: string[], after?: () => void) => {
    setBusy(true);
    setNote(null);
    try {
      await api.upsertConfig('dev_roots', next);
      setRoots(next);
      setDiscovered(d => d.filter(x => !next.includes(x)));
      after?.();
      return true;
    } catch (e) {
      setNote({ kind: 'error', text: `Couldn't save (${e instanceof Error ? e.message : String(e)}). Nothing changed.` });
      return false;
    } finally {
      setBusy(false);
    }
  };

  const add = async (raw: string) => {
    const candidate = raw.trim();
    if (!candidate) return false;
    setBusy(true);
    setNote(null);
    let checked: { resolved: string; exists: boolean; has_repositories: boolean };
    try {
      checked = await api.checkDevRoot(candidate);
    } catch (e) {
      setBusy(false);
      setNote({ kind: 'error', text: `Couldn't check that folder (${e instanceof Error ? e.message : String(e)}).` });
      return false;
    }
    if (!checked.exists) {
      setBusy(false);
      setNote({ kind: 'error', text: `There's no folder at ${checked.resolved}. It wasn't added — a path that doesn't exist would be dropped silently later.` });
      return false;
    }
    if (roots.includes(checked.resolved)) {
      setBusy(false);
      setNote({ kind: 'warn', text: `${checked.resolved} is already in the list.` });
      return false;
    }
    return await persist([...roots, checked.resolved], () => {
      setInput('');
      setNote(checked.has_repositories
        ? { kind: 'ok', text: `Added ${checked.resolved}.` }
        : { kind: 'warn', text: `Added ${checked.resolved} — I don't see any git repositories in there yet.` });
    });
  };

  const remove = (p: string) => persist(roots.filter(x => x !== p));

  const rowStyle: React.CSSProperties = {
    display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12,
    padding: '8px 0', borderBottom: `1px solid ${colors.border}`,
  };
  const pathStyle: React.CSSProperties = {
    fontFamily: font.mono, fontSize: 12, color: colors.text, wordBreak: 'break-all',
  };
  // The look these two row links had, expressed the one way the primitive can
  // also give them a press: an inline `color` would win over `.pa-btn:hover` in
  // the cascade and quietly cancel the state being migrated in for. Padding
  // stays at zero — these sit hard against the right edge of their row.
  const linkStyle = {
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-pad': '0',
    fontFamily: font.body, fontSize: 12, flexShrink: 0,
  } as CSSProperties;

  return (
    <div>
      <div style={{ fontFamily: font.body, fontSize: 12, color: colors.textMuted, marginBottom: 10, maxWidth: 620, lineHeight: 1.6 }}>
        Used to find your projects, reclaim disk space from old build caches, and
        open the right checkout. With nothing set here those features fall back to
        guessing, and a wrong guess doesn't fail loudly — it just finds nothing.
      </div>

      {loading && (
        <div style={{ fontFamily: font.body, fontSize: 12, color: colors.textMuted }}>Loading…</div>
      )}

      {!loading && roots.length === 0 && (
        <div style={{ fontFamily: font.body, fontSize: 12, color: colors.warning, marginBottom: 8 }}>
          Nothing set — features that look for your code are guessing right now.
        </div>
      )}

      {roots.map(p => (
        <div key={p} style={rowStyle}>
          <span style={pathStyle}>{p}</span>
          <Button colors={colors} variant="bare" className="hover:underline" style={linkStyle} disabled={busy} onClick={() => remove(p)}>Remove</Button>
        </div>
      ))}

      {discovered.length > 0 && (
        <div style={{ marginTop: 14 }}>
          <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textMuted, marginBottom: 4 }}>
            Also found on this machine (a git repository was actually detected in each):
          </div>
          {discovered.map(p => (
            <div key={p} style={rowStyle}>
              <span style={{ ...pathStyle, color: colors.textMuted }}>{p}</span>
              <Button colors={colors} variant="bare" className="hover:underline" style={linkStyle} disabled={busy} onClick={() => persist([...roots, p])}>Add</Button>
            </div>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 14 }}>
        <input
          value={input}
          onChange={e => { setInput(e.target.value); setNote(null); }}
          onKeyDown={e => { if (e.key === 'Enter') void add(input); }}
          placeholder={home ? `e.g. ${home}/Documents/dev` : 'e.g. ~/Documents/dev'}
          style={{
            flex: 1, height: 34, padding: '0 10px', borderRadius: radius.md,
            background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            color: colors.text, fontFamily: font.mono, fontSize: 12, outline: 'none',
          }}
        />
        <Button
          colors={colors}
          variant="ghostOn"
          onClick={() => add(input)}
          disabled={busy || !input.trim()}
          // A disabled control in Settings has to say WHY, so the guard in
          // SettingsView.preview-controls.test.tsx can still catch the thing it
          // was written for: a mockup control that is disabled forever with no
          // reason. This one is transient — type a path, or wait for the check.
          data-disabled-reason={busy ? 'checking-folder' : !input.trim() ? 'no-path-entered' : undefined}
          style={{
            '--pa-btn-fg': busy || !input.trim() ? colors.textDim : colors.cyan,
            '--pa-btn-border': `${colors.cyan}66`,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-pad': '0 14px',
            '--pa-btn-radius': `${radius.md}px`,
            '--pa-btn-weight': 600,
            height: 34, flexShrink: 0,
            fontFamily: font.body, fontSize: 12,
          } as CSSProperties}
        >{busy ? 'Checking…' : 'Add folder'}</Button>
      </div>

      {note && (
        <div
          role={note.kind === 'error' ? 'alert' : 'status'}
          style={{
            fontFamily: font.body, fontSize: 11, marginTop: 8,
            color: note.kind === 'error' ? colors.danger : note.kind === 'warn' ? colors.warning : colors.cyan,
          }}
        >{note.text}</div>
      )}
    </div>
  );
}
