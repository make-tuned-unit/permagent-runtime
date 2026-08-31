/**
 * What "Discuss with {agent}" just did.
 *
 * That button sits on a decision card and reads as "open a discussion". It is
 * also a session swap: whatever conversation was on screen — possibly
 * mid-stream, possibly mid-answer — is replaced by a fresh one. Nothing is
 * destroyed; the previous session is in Sessions. But nothing said so, and the
 * visible effect was a chat that disappeared when you pressed a button about
 * something else.
 *
 * A line, not a confirm. The ruling here is deliberate: this is a recoverable
 * side effect, and spending a full-attention interruption on it would cost more
 * than it saves. It is dismissible, and it says where the old chat went so the
 * user can act on it rather than only be reassured.
 */

import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';

export function DiscussNotice() {
  const { colors } = useTheme();
  const notice = useCommandCenter(s => s.discussNotice);
  const clear = useCommandCenter(s => s.clearDiscussNotice);
  if (!notice) return null;

  const accent = notice.tone === 'error' ? colors.danger : colors.cyan;

  return (
    <div
      data-testid="discuss-notice"
      data-tone={notice.tone}
      style={{
        flexShrink: 0,
        display: 'flex',
        alignItems: 'flex-start',
        gap: 10,
        margin: '0 12px 8px',
        padding: '8px 10px',
        borderRadius: radius.md,
        border: `1px solid ${accent}`,
        background: notice.tone === 'error' ? 'transparent' : colors.cyanSoft,
        fontFamily: font.body,
        fontSize: 11,
        lineHeight: 1.5,
        color: notice.tone === 'error' ? colors.danger : colors.textMuted,
      }}
    >
      <span style={{ flex: 1, minWidth: 0 }}>{notice.text}</span>
      <button
        type="button"
        onClick={clear}
        aria-label="Dismiss"
        style={{
          flexShrink: 0, background: 'none', border: 'none', padding: 0,
          cursor: 'pointer', color: colors.textDim, fontSize: 13, lineHeight: 1,
        }}
      >
        ×
      </button>
    </div>
  );
}
