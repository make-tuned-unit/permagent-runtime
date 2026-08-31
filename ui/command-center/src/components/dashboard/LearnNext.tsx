// LearnNext — the onboarding coach, on the Dashboard.
//
// The agent knows the app better than the user and tracks what they've actually
// engaged (server-side, from real activity). This card surfaces the single
// highest-value capability they haven't tried yet and offers to have the agent
// walk them through it — driving the real teaching loop (load_feature_lesson +
// navigate_app), not a canned tooltip.
//
// Honest + gentle: it only appears when there is a genuinely unused capability
// (the backend computes inventory − used), it shows real progress, and "Not now"
// quiets it for a few days. When the user has tried everything, it disappears.

import { useEffect, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { font, radius, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { useCommandCenter } from '../../lib/store';
import { setSpeakReplies } from '../../lib/speakReplies';

const LS_KEY = 'permagent-learn-next-state';
const DAY = 86_400_000;
const DISMISS_COOLDOWN = 3 * DAY;

interface LearnNextItem {
  id: string;
  display_name: string;
  what_it_does: string;
  why_it_matters: string;
  tab: string;
  section: string | null;
}

interface OnboardingStatus {
  used: Array<{ id: string; display_name: string }>;
  learn_next: LearnNextItem[];
  totals: { used: number; teachable: number };
}

interface DismissState {
  dismissedUntil?: number;
  lastDismissedId?: string;
  /** Capabilities the user clicked past — future loads start on a fresh one. */
  skippedIds?: string[];
}

function readState(): DismissState {
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) || '{}');
  } catch {
    return {};
  }
}
function writeState(s: DismissState) {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(s));
  } catch {
    /* private mode — best effort */
  }
}

/** First sentence (or a clamped slice) of a capability's description. */
function summarize(text: string): string {
  const firstStop = text.indexOf('. ');
  const s = firstStop > 40 ? text.slice(0, firstStop) : text;
  return s.length > 200 ? `${s.slice(0, 197)}…` : s;
}

export function LearnNext() {
  const { colors } = useTheme();
  const sendMessage = useCommandCenter(s => s.sendMessage);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const openChatDock = useCommandCenter(s => s.openChatDock);

  const [items, setItems] = useState<LearnNextItem[]>([]);
  const [idx, setIdx] = useState(0);
  const [totals, setTotals] = useState<{ used: number; teachable: number } | null>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const st = readState();
    if (st.dismissedUntil && Date.now() < st.dismissedUntil) return;

    let cancelled = false;
    (async () => {
      try {
        const status = await apiFetch<OnboardingStatus>('/api/onboarding/status');
        if (cancelled) return;
        if (status.learn_next.length > 0) {
          setItems(status.learn_next);
          setTotals(status.totals);
          // Start on a capability the user hasn't clicked past — the backend
          // ranks statically, so without this the same #1 (Decision Inbox)
          // greeted every single app open.
          const skipped = new Set(st.skippedIds ?? []);
          const fresh = status.learn_next.findIndex(i => !skipped.has(i.id));
          setIdx(fresh >= 0 ? fresh : 0);
          setVisible(true);
        }
      } catch {
        /* onboarding status unreachable — no card, no noise */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const item = items[idx] ?? null;
  if (!item || !visible) return null;

  const dismiss = () => {
    const st = readState();
    writeState({ ...st, dismissedUntil: Date.now() + DISMISS_COOLDOWN, lastDismissedId: item.id });
    setVisible(false);
  };

  // Cycle to the next unlearned capability (wraps). Remembers what was
  // clicked past so the next app open starts somewhere new.
  const nextTip = () => {
    if (items.length < 2) return;
    const st = readState();
    const skipped = new Set(st.skippedIds ?? []);
    skipped.add(item.id);
    writeState({ ...st, skippedIds: [...skipped].slice(-100) });
    setIdx((idx + 1) % items.length);
  };

  // The real teaching loop: hand the agent an explicit ask; it responds by
  // calling load_feature_lesson for this capability and navigating to its tab.
  const showMe = () => {
    // Open the dock explicitly — setActivePanel('chat') only dismisses
    // overlays, so before this the walkthrough was sent to a chat nobody
    // could see (the button looked dead) and the agent's streaming reply —
    // the immediate feedback that makes the tour feel alive — was invisible.
    setActivePanel('chat');
    openChatDock();
    // Voice-first onboarding (#18): the agent TALKS through the walkthrough;
    // the chat header's mute drops it back to text (and persists).
    setSpeakReplies(true);
    void sendMessage(
      `I haven't used ${item.display_name} yet — walk me through it and show me how it works.`,
    );
    setVisible(false);
  };

  const progress = totals ? `${totals.used}/${totals.teachable} explored` : '';

  return (
    <div
      role="note"
      aria-label={`Learn next: ${item.display_name}`}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 18,
        marginBottom: 20,
        padding: '14px 16px',
        borderRadius: 14,
        background: colors.surface,
        border: `1px solid ${colors.borderHi}`,
        boxShadow: colors.cardShadow,
        overflow: 'hidden',
      }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontFamily: font.mono,
            fontSize: 10,
            letterSpacing: '0.14em',
            color: colors.textDim,
            marginBottom: 4,
            display: 'flex',
            gap: 10,
            alignItems: 'center',
          }}
        >
          <span>✦ LEARN NEXT</span>
          {progress && <span style={{ color: colors.textMuted }}>{progress}</span>}
        </div>
        <div style={{ fontFamily: font.body, fontSize: textSize.body, color: colors.text, lineHeight: 1.4 }}>
          You haven&apos;t tried{' '}
          <span style={{ fontWeight: 700, color: colors.cyan }}>{item.display_name}</span> yet —{' '}
          <span style={{ color: colors.textMuted }}>{summarize(item.what_it_does)}.</span>
        </div>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
        {/* Every control here now rides the Button primitive. Its look arrives
            as `--pa-btn-*` custom properties, never as an inline
            `color`/`background`/`border`: an inline declaration outranks the
            `:hover` rule and would cancel the very states being adopted. */}
        {items.length > 1 && (
          <Button
            colors={colors}
            type="button"
            onClick={nextTip}
            aria-label="Show a different capability"
            title="Next tip"
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-border': colors.border,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-bg-active': colors.surface,
              '--pa-btn-pad': '7px 10px',
              '--pa-btn-radius': '9px',
              '--pa-btn-weight': 400,
              fontFamily: font.body,
              fontSize: textSize.caption,
              whiteSpace: 'nowrap',
            } as CSSProperties}
          >
            ›
          </Button>
        )}
        <Button
          colors={colors}
          type="button"
          onClick={showMe}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-border': colors.cyan,
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-fg-hover': colors.cyan,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-bg-active': colors.cyanGlow,
            '--pa-btn-pad': '7px 14px',
            '--pa-btn-radius': '9px',
            '--pa-btn-weight': 600,
            fontFamily: font.body,
            fontSize: textSize.caption,
            whiteSpace: 'nowrap',
          } as CSSProperties}
        >
          Show me
        </Button>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          onClick={dismiss}
          aria-label="Dismiss this suggestion"
          title="Not now"
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': colors.textDim,
            '--pa-btn-border': 'transparent',
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-active': colors.surface,
            '--pa-btn-pad': '0',
            '--pa-btn-radius': `${radius.md}px`,
            '--pa-btn-weight': 400,
            width: 26,
            height: 26,
            fontSize: textSize.caption,
          } as CSSProperties}
        >
          ✕
        </Button>
      </div>
    </div>
  );
}
