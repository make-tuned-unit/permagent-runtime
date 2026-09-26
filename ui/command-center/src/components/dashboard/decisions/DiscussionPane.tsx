/**
 * Ask Henry (⑧) — the inline discussion under a decision (U7).
 *
 * It used to be a permanent 5fr side column (about 41% of the sheet) whose
 * resting content was a heading, a paragraph and an empty textarea. Now it is
 * the last element of the single column: a one-line, auto-growing composer
 * with the thread growing above it (capped at 50vh). Nothing typed here
 * answers the decision — the action bar does — and the conversation stays
 * separate from the main chat (the user's D1 ruling: discussion comes to the
 * review, never decisions to chat).
 *
 * S2: messages render as React text nodes only.
 */
import { useEffect, useId, useLayoutEffect, useRef } from 'react';
import type { CSSProperties, KeyboardEvent, RefObject } from 'react';
import { useDecisionDiscussion } from '../../../lib/decisionDiscussion';
import { duration, ease, font, radius, space, textSize, type } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';

/** Grow to fit, up to about six lines, then scroll. */
const MAX_COMPOSER_PX = type.body.fontSize * 1.5 * 6;

export function DiscussionPane({ decisionId, headline, agentName, inputRef }: {
  decisionId: string;
  headline: string;
  agentName: string;
  /** Lets the review's `/` shortcut focus the composer. */
  inputRef?: RefObject<HTMLTextAreaElement>;
}) {
  const discussion = useDecisionDiscussion(decisionId, headline);
  const { colors, reduceMotion } = useTheme();
  const inputId = useId();
  const list = useRef<HTMLDivElement>(null);
  const ownRef = useRef<HTMLTextAreaElement>(null);
  const textarea = inputRef ?? ownRef;
  const busy = discussion.status === 'creating' || discussion.status === 'sending';
  useEffect(() => {
    const element = list.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [decisionId, discussion.messages]);
  useLayoutEffect(() => {
    const el = textarea.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight || 0, MAX_COMPOSER_PX) || ''}px`;
  }, [discussion.draft, textarea]);
  const containNavigation = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target;
    if (!(target instanceof HTMLElement) ||
      !(target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target.isContentEditable)) return;
    if (['j', 'k', 'J', 'K', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
      event.stopPropagation();
    }
  };
  const hasThread = discussion.messages.length > 0;
  return (
    <section aria-label={`Ask ${agentName} about this decision`} style={{
      display: 'flex', flexDirection: 'column', gap: space.md, minWidth: 0, fontFamily: font.body,
    }}>
      {hasThread && (
        <div ref={list} role="log" aria-label="Decision discussion" aria-live="polite" style={{
          display: 'flex', flexDirection: 'column', gap: space.md, overflowY: 'auto',
          maxHeight: '50vh', overflowWrap: 'anywhere',
        }}>
          {discussion.messages.map(message => (
            <div key={message.id} className="pa-enter" style={{
              alignSelf: message.role === 'user' ? 'flex-end' : 'flex-start', maxWidth: '85%',
              borderRadius: radius.md, padding: `${space.md}px ${space.lg}px`, whiteSpace: 'pre-wrap',
              background: message.role === 'user' ? colors.surfaceHi : colors.fillSubtle,
              color: colors.text, ...type.small,
            }}>
              <span style={{ display: 'block', color: colors.textDim, ...type.micro, marginBottom: space.xxs }}>
                {message.role === 'user' ? 'You' : agentName}
              </span>
              {message.text}
              {message.streaming && <span aria-label="Assistant is responding"> …</span>}
            </div>
          ))}
        </div>
      )}
      {discussion.status === 'error' && (
        <div role="alert" style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md, ...type.small }}>
          <span style={{ color: colors.danger, overflowWrap: 'anywhere' }}>{discussion.error}</span>
          <Button colors={colors} variant="ghost" flashSuccess={false} onClick={() => { void discussion.retry(); }}>Retry</Button>
        </div>
      )}
      <form
        onSubmit={event => { event.preventDefault(); if (!busy) void discussion.send(); }}
        style={{
          display: 'flex', alignItems: 'flex-end', gap: space.md,
          padding: `${space.sm}px ${space.sm}px ${space.sm}px ${space.xl}px`,
          background: colors.inputBg, border: `1px solid ${colors.border}`, borderRadius: radius.md,
          transition: reduceMotion ? 'none' : `border-color ${duration.fast}ms ${ease.out}`,
        }}
      >
        <label htmlFor={inputId} className="sr-only">Message about this decision</label>
        <textarea
          id={inputId}
          ref={textarea}
          rows={1}
          value={discussion.draft}
          disabled={busy}
          placeholder={`Ask ${agentName} about this…`}
          onChange={event => discussion.setDraft(event.target.value)}
          onFocus={event => { (event.currentTarget.parentElement as HTMLElement).style.borderColor = colors.borderHi; }}
          onBlur={event => { (event.currentTarget.parentElement as HTMLElement).style.borderColor = colors.border; }}
          onKeyDown={event => {
            containNavigation(event);
            if (event.key === 'Enter') {
              event.stopPropagation();
              if (!event.shiftKey && !event.nativeEvent.isComposing && !event.ctrlKey && !event.altKey && !event.metaKey) {
                event.preventDefault();
                if (!busy) void discussion.send();
              }
            }
          }}
          style={{
            flex: 1, boxSizing: 'border-box', minWidth: 0, resize: 'none', border: 'none', outline: 'none',
            background: 'transparent', color: colors.text, fontFamily: font.body, ...type.body,
            padding: `${space.xs}px 0`, maxHeight: MAX_COMPOSER_PX, overflowY: 'auto',
          }}
        />
        <Button
          colors={colors}
          variant="bare"
          type="submit"
          aria-label="Send message about this decision"
          disabled={busy || !discussion.draft.trim()}
          flashSuccess={false}
          style={{
            '--pa-btn-pad': `${space.xs}px ${space.md}px`,
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': colors.fillHover,
            '--pa-btn-bg-active': colors.fillActive,
            fontFamily: font.mono, fontSize: textSize.caption,
          } as CSSProperties}
        >
          {discussion.status === 'creating' ? 'Connecting…' : '↩'}
        </Button>
      </form>
    </section>
  );
}
