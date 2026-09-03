import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { relativeTimeAgo } from '../../lib/time-decay';
import { isSafeHttpUrl } from '../../lib/url';
import { font, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import type { Project } from './types';

import { Tooltip } from '../common/Tooltip';
export interface ProjectIntelItem {
  id: string;
  kind: 'competitor' | 'partner' | 'adjacent';
  name: string;
  note: string | null;
  source_url: string;
  created_at: string;
}

interface ProjectIntelResponse {
  competitors: ProjectIntelItem[];
  partners: ProjectIntelItem[];
  ecosystem: ProjectIntelItem[];
}

const emptyIntel: ProjectIntelResponse = { competitors: [], partners: [], ecosystem: [] };

export function EcosystemPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const [intel, setIntel] = useState<ProjectIntelResponse>(emptyIntel);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [requested, setRequested] = useState(false);
  const agentName = useCommandCenter(s => s.agentName);
  const loadGeneration = useRef(0);

  // Resolves `false` when the load failed (or was superseded) so the retry
  // button can only tick over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setStatus('loading');
    try {
      const response = await apiFetch<Partial<ProjectIntelResponse>>(
        `/api/projects/${encodeURIComponent(project.id)}/intel`,
      );
      if (generation !== loadGeneration.current) return false;
      if (!response || typeof response !== 'object') throw new Error('Invalid intelligence response');
      setIntel({
        competitors: Array.isArray(response?.competitors) ? response.competitors : [],
        partners: Array.isArray(response?.partners) ? response.partners : [],
        ecosystem: Array.isArray(response?.ecosystem) ? response.ecosystem : [],
      });
      setStatus('ready');
      setRequested(false); // fresh rows landed — the button is usable again
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      setStatus('error');
      return false;
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  const dismiss = async (itemId: string) => {
    try {
      await apiFetch(
        `/api/projects/${encodeURIComponent(project.id)}/intel/${encodeURIComponent(itemId)}`,
        { method: 'DELETE' },
      );
      setIntel(current => ({
        competitors: current.competitors.filter(item => item.id !== itemId),
        partners: current.partners.filter(item => item.id !== itemId),
        ecosystem: current.ecosystem.filter(item => item.id !== itemId),
      }));
      return true;
    } catch {
      // Keep the item visible when the server did not confirm deletion — and
      // resolve `false` so the dismiss button cannot tick over a failed DELETE.
      return false;
    }
  };

  // Run it, don't hand the user a prompt (2026-07-27 ruling): the button sends
  // the research ask straight to Henry in the chat dock. Findings land through
  // propose_project_intel -> Decision Inbox approval -> intel rows, and this
  // panel refreshes via projectsRev when they do.
  const requestIntelligence = () => {
    const { setActivePanel, openChatDock, sendMessage } = useCommandCenter.getState();
    setActivePanel('chat');
    openChatDock();
    void sendMessage(
      `Refresh project intelligence for "${project.name}": call research_project_intel for ` +
      `this project, research its competitors, partners, and adjacent ecosystem with your ` +
      `web tools, then call propose_project_intel so I can review the findings in the ` +
      `Decision Inbox.`,
    );
    setRequested(true);
  };

  const groups: Array<[string, ProjectIntelItem[]]> = [
    ['Competitors', intel.competitors],
    ['Partners', intel.partners],
    ['Ecosystem', intel.ecosystem],
  ];
  const newestCreatedAt = groups
    .flatMap(([, items]) => items)
    .map(item => item.created_at)
    .filter(createdAt => !Number.isNaN(new Date(createdAt).getTime()))
    .sort((a, b) => new Date(b).getTime() - new Date(a).getTime())[0];
  const freshness = newestCreatedAt
    ? relativeTimeAgo(newestCreatedAt) || 'just now'
    : null;

  return (
    <Panel
      title="Ecosystem intelligence"
      action={(
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={requestIntelligence}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 'inherit',
            fontFamily: font.body,
            fontSize: textSize.micro,
          } as CSSProperties}
        >
          {requested ? `${agentName} is researching…` : 'Refresh intelligence'}
        </Button>
      )}
    >
      {status === 'loading' && <div style={{ color: colors.textDim, fontSize: textSize.micro }}>Loading intelligence…</div>}
      {status === 'error' && (
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={load}
          style={{
            '--pa-btn-fg': colors.danger,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 'inherit',
            fontSize: 'inherit',
            lineHeight: 'inherit',
          } as CSSProperties}
        >
          Couldn't load intelligence. Retry
        </Button>
      )}
      {status === 'ready' && groups.every(([, items]) => items.length === 0) && (
        <div style={{ color: colors.textDim, fontSize: textSize.micro }}>No researched intelligence yet.</div>
      )}
      {status === 'ready' && freshness && (
        <div style={{ color: colors.textDim, fontSize: 10 }}>Last researched {freshness}</div>
      )}
      {status === 'ready' && groups.map(([label, items]) => items.length > 0 && (
        <section key={label} style={{ marginTop: space.lg }}>
          <div style={{ color: colors.textMuted, fontSize: 10, fontWeight: 700, letterSpacing: '0.08em', textTransform: 'uppercase', marginBottom: space.sm }}>
            {label}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
            {items.map(item => (
              <div key={item.id} style={{ borderLeft: `2px solid ${colors.cyan}`, paddingLeft: space.md }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md }}>
                  <div style={{ color: colors.text, fontSize: textSize.caption, fontWeight: 600 }}>{item.name}</div>
                  <Tooltip content="Dismiss">
                    <Button
                      colors={colors}
                      variant="bare"
                      type="button"
                      aria-label={`Dismiss ${item.name}`}
                      onClick={() => dismiss(item.id)}
                      style={{
                        '--pa-btn-fg': colors.textDim,
                        '--pa-btn-fg-hover': colors.danger,
                        '--pa-btn-bg-hover': 'transparent',
                        '--pa-btn-pad': '2px',
                        '--pa-btn-weight': 'inherit',
                        fontSize: textSize.body,
                        lineHeight: 1,
                      } as CSSProperties}
                    >
                      ×
                    </Button>
                  </Tooltip>
                </div>
                {item.note && <div style={{ color: colors.textMuted, fontSize: textSize.micro, marginTop: space.xxs }}>{item.note}</div>}
                <div style={{ display: 'flex', gap: space.md, alignItems: 'center', marginTop: space.xxs, fontSize: 10 }}>
                  {isSafeHttpUrl(item.source_url) ? (
                    <a href={item.source_url} target="_blank" rel="noreferrer" style={{ color: colors.cyan }}>Source</a>
                  ) : (
                    <span style={{ color: colors.textMuted }}>Source</span>
                  )}
                  <span style={{ color: colors.textDim }}>{new Date(item.created_at).toLocaleDateString()}</span>
                </div>
              </div>
            ))}
          </div>
        </section>
      ))}
    </Panel>
  );
}
