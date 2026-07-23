import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { navigateToTool } from '../../lib/store';
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
  const [promptCopied, setPromptCopied] = useState(false);

  const load = useCallback(async () => {
    setStatus('loading');
    try {
      const response = await apiFetch<Partial<ProjectIntelResponse>>(
        `/api/projects/${encodeURIComponent(project.id)}/intel`,
      );
      setIntel({
        competitors: Array.isArray(response?.competitors) ? response.competitors : [],
        partners: Array.isArray(response?.partners) ? response.partners : [],
        ecosystem: Array.isArray(response?.ecosystem) ? response.ecosystem : [],
      });
      setStatus('ready');
    } catch {
      setStatus('error');
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  // Prepared-prompt pattern mirrored from PersonDetailModal's Enricher flow.
  const requestIntelligence = () => {
    const prompt =
      `Refresh project intelligence for ${project.name}: call research_project_intel with ` +
      `project "${project.name}", research its competitors, partners, and adjacent ecosystem ` +
      `with your web tools, then call propose_project_intel so I can review the findings in ` +
      `the Decision Inbox.`;
    navigator.clipboard?.writeText(prompt).catch(() => {});
    setPromptCopied(true);
    navigateToTool('chat');
  };

  const groups: Array<[string, ProjectIntelItem[]]> = [
    ['Competitors', intel.competitors],
    ['Partners', intel.partners],
    ['Ecosystem', intel.ecosystem],
  ];

  return (
    <Panel
      title="Ecosystem intelligence"
      action={(
        <button
          type="button"
          onClick={requestIntelligence}
          style={{ border: 'none', background: 'none', color: colors.cyan, cursor: 'pointer', fontFamily: font.body, fontSize: 11, padding: 0 }}
        >
          {promptCopied ? 'Prompt copied — paste it in chat' : 'Refresh intelligence'}
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
      {status === 'ready' && groups.map(([label, items]) => items.length > 0 && (
        <section key={label} style={{ marginTop: 10 }}>
          <div style={{ color: colors.textMuted, fontSize: 10, fontWeight: 700, letterSpacing: '0.08em', textTransform: 'uppercase', marginBottom: 6 }}>
            {label}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
            {items.map(item => (
              <div key={item.id} style={{ borderLeft: `2px solid ${colors.cyan}`, paddingLeft: 9 }}>
                <div style={{ color: colors.text, fontSize: 12, fontWeight: 600 }}>{item.name}</div>
                {item.note && <div style={{ color: colors.textMuted, fontSize: 11, marginTop: 2 }}>{item.note}</div>}
                <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 3, fontSize: 10 }}>
                  <a href={item.source_url} target="_blank" rel="noreferrer" style={{ color: colors.cyan }}>Source</a>
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
