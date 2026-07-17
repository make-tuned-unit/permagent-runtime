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

import { useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { font } from '../../styles/tokens';
import { useCommandCenter } from '../../lib/store';

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

  const [item, setItem] = useState<LearnNextItem | null>(null);
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
        const top = status.learn_next[0];
        if (top) {
          setItem(top);
          setTotals(status.totals);
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

  if (!item || !visible) return null;

  const dismiss = () => {
    writeState({ dismissedUntil: Date.now() + DISMISS_COOLDOWN, lastDismissedId: item.id });
    setVisible(false);
  };

  // The real teaching loop: hand the agent an explicit ask; it responds by
  // calling load_feature_lesson for this capability and navigating to its tab.
  const showMe = () => {
    setActivePanel('chat');
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
        <div style={{ fontFamily: font.body, fontSize: 14, color: colors.text, lineHeight: 1.4 }}>
          You haven&apos;t tried{' '}
          <span style={{ fontWeight: 700, color: colors.cyan }}>{item.display_name}</span> yet —{' '}
          <span style={{ color: colors.textMuted }}>{summarize(item.what_it_does)}.</span>
        </div>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
        <button
          onClick={showMe}
          style={{
            padding: '7px 14px',
            borderRadius: 9,
            border: `1px solid ${colors.cyan}`,
            background: colors.cyanSoft,
            color: colors.cyan,
            fontFamily: font.body,
            fontSize: 12,
            fontWeight: 600,
            cursor: 'pointer',
            whiteSpace: 'nowrap',
          }}
        >
          Show me
        </button>
        <button
          onClick={dismiss}
          aria-label="Dismiss this suggestion"
          title="Not now"
          style={{
            width: 26,
            height: 26,
            display: 'grid',
            placeItems: 'center',
            borderRadius: 8,
            border: 'none',
            background: 'transparent',
            color: colors.textDim,
            fontSize: 12,
            cursor: 'pointer',
          }}
        >
          ✕
        </button>
      </div>
    </div>
  );
}
