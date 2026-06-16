import { useState } from 'react';
import { versionBannerState, type VersionSkew } from '../../lib/version';

/**
 * Non-blocking banner surfaced when the desktop app and daemon versions drift.
 * Minor skew = amber advisory; major skew = red warning (likely incompatible
 * API surface). Patch/match/unknown render nothing. Dismissible per session.
 * The show/severity/message decision lives in versionBannerState (pure, tested).
 */
export function VersionSkewBanner({ skew }: { skew: VersionSkew | null }) {
  const [dismissed, setDismissed] = useState(false);

  const state = versionBannerState(skew);
  if (!state || dismissed) return null;

  const major = state.severity === 'major';
  const bg = major ? 'rgba(180,32,42,0.16)' : 'rgba(196,138,32,0.14)';
  const fg = major ? '#f3a3a3' : '#e7c46a';
  const border = major ? 'rgba(180,32,42,0.45)' : 'rgba(196,138,32,0.4)';

  return (
    <div
      role="status"
      className="flex items-center justify-between gap-3 px-3 py-1.5 text-xs font-mono"
      style={{ flexShrink: 0, background: bg, color: fg, borderBottom: `1px solid ${border}` }}
    >
      <span className="truncate">{state.message}</span>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        className="shrink-0 opacity-70 hover:opacity-100"
        style={{ color: fg }}
        aria-label="Dismiss version notice"
      >
        ✕
      </button>
    </div>
  );
}
