/**
 * Governance — one sovereign control surface over your own machine. Consolidates
 * four things that already existed, scattered, into one legible view, each panel
 * wired to a REAL mounted endpoint:
 *
 *   Models       — active model per role + roster       (/config, /api/agent/workers)
 *   Spend        — per-session/project cost + budget     (/api/governance/spend|budget)
 *   Sovereignty  — data boundary + egress audit log      (/api/security/*)
 *   Approvals    — pending decisions + approval posture  (/api/decisions, /config)
 *
 * Rendered as an overlay when activePanel === 'governance' (agent-navigable via
 * navigate_app("Governance")). Enforced locally — not by a cloud admin.
 */

import { useCallback, useEffect, useState } from 'react';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { ModelsPanel } from './ModelsPanel';
import { SpendPanel } from './SpendPanel';
import { SovereigntyPanel } from './SovereigntyPanel';
import { ApprovalsPanel } from './ApprovalsPanel';

type TabId = 'spend' | 'sovereignty' | 'models' | 'approvals';

const TABS: { id: TabId; label: string }[] = [
  { id: 'spend', label: 'Spend' },
  { id: 'sovereignty', label: 'Sovereignty' },
  { id: 'models', label: 'Models' },
  { id: 'approvals', label: 'Approvals' },
];

export function GovernanceView() {
  const { gradient, colors } = useTheme();
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [tab, setTab] = useState<TabId>('spend');

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  useEffect(() => {
    const h = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); dismiss(); } };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [dismiss]);

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.shell, color: colors.text, fontFamily: font.body }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '20px 32px', borderBottom: `1px solid ${colors.border}` }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, letterSpacing: '-0.01em' }}>Governance</div>
          <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 2 }}>
            What you run, what it costs, what has left this machine, and what needs your sign-off — enforced locally, not by a cloud admin.
          </div>
        </div>
        <button
          onClick={dismiss}
          style={{ height: 30, padding: '0 12px', borderRadius: 8, background: 'transparent', border: `1px solid ${colors.border}`, color: colors.textMuted, cursor: 'pointer', fontFamily: font.body, fontSize: 12 }}
        >Close</button>
      </div>

      {/* Sub-nav */}
      <div role="tablist" aria-label="Governance sections" style={{ display: 'flex', gap: 4, padding: '12px 32px 0', borderBottom: `1px solid ${colors.border}` }}>
        {TABS.map(t => {
          const active = tab === t.id;
          return (
            <button
              key={t.id}
              role="tab"
              aria-selected={active}
              onClick={() => setTab(t.id)}
              style={{
                height: 34, padding: '0 16px', borderRadius: '8px 8px 0 0',
                background: 'transparent', border: 'none',
                borderBottom: `2px solid ${active ? colors.cyan : 'transparent'}`,
                color: active ? colors.text : colors.textMuted,
                cursor: 'pointer', fontFamily: font.body, fontSize: 13, fontWeight: active ? 600 : 500,
              }}
            >
              {t.label}
            </button>
          );
        })}
      </div>

      {/* Panel body */}
      <div style={{ flex: 1, overflow: 'auto', padding: '24px 32px', maxWidth: 900, width: '100%', margin: '0 auto', boxSizing: 'border-box' }}>
        {tab === 'spend' && <SpendPanel />}
        {tab === 'sovereignty' && <SovereigntyPanel />}
        {tab === 'models' && <ModelsPanel />}
        {tab === 'approvals' && <ApprovalsPanel />}
      </div>
    </div>
  );
}
