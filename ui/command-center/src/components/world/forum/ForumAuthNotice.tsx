import { useState, type FormEvent } from 'react';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';
import { space } from '../../../styles/tokens';
import { useCommandCenter } from '../../../lib/store';
import { canPairThisBrowser, pairThisBrowser } from '../../../lib/api';

/**
 * Why the World cannot reach the agent, and the way to fix it, on the World.
 *
 * 2026-09-19: the standalone forum page (localhost:5173/ui/forum.html) open in
 * an unpaired Chrome tab stopped flooding the daemon with 401s once the api
 * client held unpaired requests back — but the reason only reached the store
 * and the console. `daemonAuthError` was drawn by the chat's MessageList,
 * which this page never mounts, so the forum simply went quiet ("Job feed
 * unavailable / connecting", forever) with nothing saying it was not paired.
 *
 * Pairing is offered right here because the link from Settings → Devices is
 * built on the hub origin, and opening it pairs THAT origin's storage, never
 * this tab's. Pasting it redeems the one-time code through this page's own
 * origin instead (`pairThisBrowser`).
 */
export function ForumAuthNotice({compact=false}:{compact?:boolean}) {
  const [expanded,setExpanded]=useState(false);
  const authError = useCommandCenter(s => s.daemonAuthError);
  const { colors } = useTheme();
  const [link, setLink] = useState('');
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<string | null>(null);

  if (!authError) return null;
  const canPair = canPairThisBrowser();

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!link.trim() || busy) return;
    setBusy(true);
    setOutcome(null);
    try {
      const result = await pairThisBrowser(link);
      // On success the alert disappears with `daemonAuthError`; on failure the
      // published message already says why, so only a parse failure (which
      // publishes nothing) needs its own line.
      setOutcome(result.paired || result.message === authError ? null : result.message);
      if (result.paired) setLink('');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className={`forum-auth-notice ${compact?'habitat-auth':''}`}
      role="alert"
      style={{
        pointerEvents: 'auto',
        maxWidth: 420,
        // On the spacing scale (spaceScale.test.ts freezes off-scale literals
        // per directory; components/world had no budget left for 14/18).
        padding: `${space.xl}px ${space.xxl}px`,
        background: 'var(--forum-bg)',
        border: `1px solid ${colors.danger}`,
        borderRadius: 'var(--forum-radius)',
        boxShadow: '0 8px 24px #0002',
      }}
    >
      <b style={{ color: colors.danger }}>Not connected to your agent</b>{compact&&<button className="habitat-auth-toggle" onClick={()=>setExpanded(v=>!v)} aria-expanded={expanded}>{expanded?'Hide details':'Connect…'}</button>}
      {(!compact||expanded)&&<>
      <p style={{ margin: `${space.sm}px 0 0` }}>{authError}</p>
      {canPair && (
        <form onSubmit={submit} style={{ display: 'flex', flexDirection: 'column', gap: space.sm, marginTop: space.lg }}>
          <label className="field-label" htmlFor="forum-pair-link">
            Paste the pairing link from Settings → Devices to pair this tab
          </label>
          <div style={{ display: 'flex', gap: space.md }}>
            <input
              id="forum-pair-link"
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={link}
              placeholder="http://…/ui/#claim=…"
              onChange={e => setLink(e.target.value)}
              style={{ flex: 1, minWidth: 0 }}
            />
            <Button colors={colors} variant="primary" flashSuccess={false} type="submit" pending={busy} disabled={busy || !link.trim()}>
              Pair this tab
            </Button>
          </div>
          {outcome && <p role="status" style={{ margin: 0 }}>{outcome}</p>}
        </form>
      )}
      </>}
    </div>
  );
}
