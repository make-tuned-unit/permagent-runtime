import { useCallback, useEffect, useRef, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { relativeTimeAgo } from '../../lib/time-decay';
import { isSafeHttpUrl } from '../../lib/url';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Panel } from './Panel';
import type { Project } from './types';

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
  const loadGeneration = useRef(0);

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setStatus('loading');
    try {
      const response = await apiFetch<Partial<ProjectIntelResponse>>(
        `/api/projects/${encodeURIComponent(project.id)}/intel`,
      );
      if (generation !== loadGeneration.current) return;
      if (!response || typeof response !== 'object') throw new Error('Invalid intelligence response');
      setIntel({
        competitors: Array.isArray(response?.competitors) ? response.competitors : [],
        partners: Array.isArray(response?.partners) ? response.partners : [],
        ecosystem: Array.isArray(response?.ecosystem) ? response.ecosystem : [],
      });
      setStatus('ready');
      setRequested(false); // fresh rows landed — the button is usable again
    } catch {
      if (generation !== loadGeneration.current) return;
      setStatus('error');
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
    } catch {
      // Keep the item visible when the server did not confirm deletion.
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
        <button
          type="button"
          onClick={requestIntelligence}
          style={{ border: 'none', background: 'none', color: colors.cyan, cursor: 'pointer', fontFamily: font.body, fontSize: 11, padding: 0 }}
        >
          {requested ? 'Henry is researching…' : 'Refresh intelligence'}
        </button>
      )}
    >
      {status === 'loading' && <div style={{ color: colors.textDim, fontSize: 11 }}>Loading intelligence…</div>}
      {status === 'error' && (
        <button type="button" onClick={load} style={{ border: 'none', background: 'none', color: colors.danger, cursor: 'pointer', padding: 0 }}>
          Couldn't load intelligence. Retry
        </button>
      )}
      {status === 'ready' && groups.every(([, items]) => items.length === 0) && (
        <div style={{ color: colors.textDim, fontSize: 11 }}>No researched intelligence yet.</div>
      )}
      {status === 'ready' && freshness && (
        <div style={{ color: colors.textDim, fontSize: 10 }}>Last researched {freshness}</div>
      )}
      {status === 'ready' && groups.map(([label, items]) => items.length > 0 && (
        <section key={label} style={{ marginTop: 10 }}>
          <div style={{ color: colors.textMuted, fontSize: 10, fontWeight: 700, letterSpacing: '0.08em', textTransform: 'uppercase', marginBottom: 6 }}>
            {label}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
            {items.map(item => (
              <div key={item.id} style={{ borderLeft: `2px solid ${colors.cyan}`, paddingLeft: 9 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                  <div style={{ color: colors.text, fontSize: 12, fontWeight: 600 }}>{item.name}</div>
                  <button
                    type="button"
                    aria-label={`Dismiss ${item.name}`}
                    title="Dismiss"
                    onClick={() => dismiss(item.id)}
                    style={{ border: 'none', background: 'none', color: colors.textDim, cursor: 'pointer', fontSize: 14, lineHeight: 1, padding: 2 }}
                  >
                    ×
                  </button>
                </div>
                {item.note && <div style={{ color: colors.textMuted, fontSize: 11, marginTop: 2 }}>{item.note}</div>}
                <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 3, fontSize: 10 }}>
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
