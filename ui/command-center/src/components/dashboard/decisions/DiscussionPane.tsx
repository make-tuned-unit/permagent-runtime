import { useEffect, useId, useRef } from 'react';
import type { KeyboardEvent } from 'react';
import { useDecisionDiscussion } from '../../../lib/decisionDiscussion';
import { radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';

export function DiscussionPane({ decisionId, headline, agentName }: {
  decisionId: string;
  headline: string;
  agentName: string;
}) {
  const discussion = useDecisionDiscussion(decisionId, headline);
  const { colors } = useTheme();
  const inputId = useId();
  const headingId = useId();
  const list = useRef<HTMLDivElement>(null);
  const busy = discussion.status === 'creating' || discussion.status === 'sending';
  useEffect(() => {
    const element = list.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [decisionId, discussion.messages]);
  const containNavigation = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target;
    if (!(target instanceof HTMLElement) ||
      !(target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target.isContentEditable)) return;
    if (['j', 'k', 'J', 'K', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
      event.stopPropagation();
    }
  };
  return (
    <section aria-labelledby={headingId} style={{
      display: 'flex', flexDirection: 'column', gap: space.xl, minWidth: 0, minHeight: 0,
      maxHeight: '100%', padding: space.xxl, background: colors.surface, color: colors.text,
      borderRadius: radius.md, overflow: 'hidden', fontSize: textSize.body,
    }}>
      <header>
        <h3 id={headingId} style={{ margin: 0, fontSize: textSize.heading }}>Talk it through with {agentName}</h3>
      </header>
      <div ref={list} role="log" aria-label="Decision discussion" aria-live="polite" style={{
        display: 'flex', flexDirection: 'column', gap: space.md, overflowY: 'auto',
        minHeight: 0, maxHeight: '50vh', flex: '1 1 auto', overflowWrap: 'anywhere',
      }}>
        {discussion.messages.length === 0 && (
          <p style={{ margin: 0, color: colors.textMuted }}>
            Ask about this decision before you answer. This conversation stays separate from your main chat, and
            nothing typed here answers the decision — the buttons beside it do.
          </p>
        )}
        {discussion.messages.map(message => (
          <div key={message.id} style={{
            alignSelf: message.role === 'user' ? 'flex-end' : 'flex-start', maxWidth: '90%',
            borderRadius: radius.md, padding: space.md, whiteSpace: 'pre-wrap',
            background: message.role === 'user' ? colors.surfaceHi : colors.bg,
          }}>
            <span style={{ display: 'block', color: colors.textMuted, fontSize: textSize.caption }}>
              {message.role === 'user' ? 'You' : agentName}
            </span>
            {message.text}
            {message.streaming && <span aria-label="Assistant is responding"> …</span>}
          </div>
        ))}
      </div>
      {discussion.status === 'error' && (
        <div role="alert" style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md }}>
          <span style={{ color: colors.danger, overflowWrap: 'anywhere' }}>{discussion.error}</span>
          <Button colors={colors} flashSuccess={false} onClick={() => { void discussion.retry(); }}>Retry</Button>
        </div>
      )}
      <form onSubmit={event => { event.preventDefault(); if (!busy) void discussion.send(); }} style={{
        display: 'flex', flexDirection: 'column', gap: space.md,
      }}>
        <label htmlFor={inputId} style={{ fontSize: textSize.caption }}>Message about this decision</label>
        <textarea id={inputId} rows={3} value={discussion.draft} disabled={busy}
          onChange={event => discussion.setDraft(event.target.value)}
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
            boxSizing: 'border-box', width: '100%', minWidth: 0, resize: 'vertical',
            maxHeight: '25vh', borderRadius: radius.sm, padding: space.md,
            background: colors.bg, color: colors.text, border: `1px solid ${colors.border}`,
            fontFamily: 'inherit', fontSize: textSize.body,
          }}
        />
        <Button
          colors={colors}
          type="submit"
          aria-label="Send message about this decision"
          disabled={busy || !discussion.draft.trim()}
          flashSuccess={false}
        >
          {discussion.status === 'creating' ? 'Connecting…' : 'Send'}
        </Button>
      </form>
    </section>
  );
}
