import { useEffect, useState, useCallback } from 'react';
import { color, font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';

interface ActivityMemory {
  id: string;
  key: string;
  content: string;
  wing: string | null;
  compaction_tier: string | null;
  created_at: string | null;
}

interface DigestData {
  live_state: {
    active_project_id: string | null;
    active_session_id: string | null;
    last_browser_url: string | null;
    last_terminal_command: string | null;
    last_terminal_cwd: string | null;
    events_in_last_5_minutes: number;
  };
  recent_events: unknown[];
  probed_memories: Array<{
    content: string;
    relevance: number;
    wing: string | null;
  }>;
  recalled_memories: Array<{
    content: string;
    signal_score: number;
  }>;
}

interface IngestStatus {
  active_project?: {
    project_id: string;
    project_name: string;
    wing: string;
  } | null;
  paused: boolean;
  events_ingested: { always: number; aggregated: number };
  events_observed: { ephemeral: number };
}

interface Props {
  onClose: () => void;
}

export function InspectionPanel({ onClose }: Props) {
  const pushOverlay = useCommandCenter(s => s.pushBrowserOverlay);
  const popOverlay = useCommandCenter(s => s.popBrowserOverlay);
  useEffect(() => { pushOverlay(); return () => { popOverlay(); }; }, [pushOverlay, popOverlay]);

  const [status, setStatus] = useState<IngestStatus | null>(null);
  const [digest, setDigest] = useState<DigestData | null>(null);
  const [memories, setMemories] = useState<ActivityMemory[]>([]);
  const [events, setEvents] = useState<unknown[]>([]);
  const [paused, setPaused] = useState(false);
  const [expandedMemory, setExpandedMemory] = useState<string | null>(null);
  const [expandedEvent, setExpandedEvent] = useState<number | null>(null);
  const [memoriesOpen, setMemoriesOpen] = useState(false);
  const [digestOpen, setDigestOpen] = useState(false);
  const [sourceFilters, setSourceFilters] = useState<Record<string, boolean>>({
    chat: true, browser: true, terminal: true, project: true, skills: true, integrations: true,
  });

  const fetchStatus = useCallback(async () => {
    try {
      const res = await api.fetchAuthed('/activity/ingest-status');
      const data = await res.json();
      setStatus(data);
      setPaused(data.paused ?? false);
    } catch { /* ignore */ }
  }, []);

  const fetchDigest = useCallback(async () => {
    try {
      const res = await api.fetchAuthed('/activity/current-digest');
      const data = await res.json();
      setDigest(data);
    } catch { /* ignore */ }
  }, []);

  const fetchMemories = useCallback(async () => {
    try {
      const res = await api.fetchAuthed('/activity/recent-memories?limit=20');
      const data = await res.json();
      setMemories(data.memories ?? []);
    } catch { /* ignore */ }
  }, []);

  const fetchRecentEvents = useCallback(async () => {
    try {
      const res = await api.fetchAuthed('/activity/recent?limit=200');
      const data = await res.json();
      setEvents(data ?? []);
    } catch { /* ignore */ }
  }, []);

  useEffect(() => {
    fetchStatus();
    fetchDigest();
    fetchMemories();
    fetchRecentEvents();

    const interval = setInterval(() => {
      fetchDigest();
      fetchRecentEvents();
    }, 10000);

    return () => clearInterval(interval);
  }, [fetchStatus, fetchDigest, fetchMemories, fetchRecentEvents]);

  const togglePause = async () => {
    try {
      const endpoint = paused ? '/activity/resume' : '/activity/pause';
      await api.fetchAuthed(endpoint, { method: 'POST' });
      setPaused(!paused);
    } catch { /* ignore */ }
  };

  const toggleFilter = (key: string) => {
    setSourceFilters(prev => ({ ...prev, [key]: !prev[key] }));
  };

  const filteredEvents = events.filter((e: any) => {
    const surface = (e.source_surface ?? '').toLowerCase();
    if (surface.includes('chat') && !sourceFilters.chat) return false;
    if (surface.includes('browser') && !sourceFilters.browser) return false;
    if (surface.includes('terminal') && !sourceFilters.terminal) return false;
    if (surface.includes('project') && !sourceFilters.project) return false;
    if (surface.includes('skill') && !sourceFilters.skills) return false;
    if (surface.includes('integration') && !sourceFilters.integrations) return false;
    return true;
  });

  return (
    <div style={{
      position: 'fixed', top: 0, right: 0, bottom: 0,
      width: 400, zIndex: 10000,
      background: 'rgba(8,14,26,0.98)', backdropFilter: 'blur(24px)',
      borderLeft: `1px solid ${color.borderHi}`,
      display: 'flex', flexDirection: 'column',
      fontFamily: font.body, fontSize: 12,
      boxShadow: '-8px 0 32px rgba(0,0,0,0.4)',
    }}>
      {/* Header */}
      <div style={{
        padding: '12px 16px', borderBottom: `1px solid ${color.border}`,
        display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0,
      }}>
        <span style={{ flex: 1, fontWeight: 600, fontSize: 13, color: color.text }}>
          What your agent sees
        </span>
        <button onClick={togglePause} style={{
          padding: '4px 10px', borderRadius: 4, fontSize: 11, fontWeight: 500,
          background: paused ? 'rgba(255,180,0,0.12)' : 'rgba(0,213,255,0.08)',
          border: `1px solid ${paused ? 'rgba(255,180,0,0.3)' : color.border}`,
          color: paused ? '#ffb400' : color.cyan, cursor: 'pointer',
        }}>
          {paused ? 'Resume' : 'Pause'}
        </button>
        <a href="/brain" target="_blank" rel="noopener" style={{
          padding: '4px 10px', borderRadius: 4, fontSize: 11, fontWeight: 500,
          background: 'rgba(255,255,255,0.04)', border: `1px solid ${color.border}`,
          color: color.textMuted, textDecoration: 'none', cursor: 'pointer',
        }}>
          Open Brain
        </a>
        <button onClick={onClose} style={{
          width: 24, height: 24, borderRadius: 4,
          background: 'rgba(255,255,255,0.04)', border: `1px solid ${color.border}`,
          color: color.textMuted, cursor: 'pointer',
          display: 'grid', placeItems: 'center',
        }}>
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Active project */}
      <div style={{ padding: '8px 16px', borderBottom: `1px solid ${color.border}`, flexShrink: 0 }}>
        {status?.active_project ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ color: color.textMuted }}>Project:</span>
            <span style={{ color: color.text, fontWeight: 500 }}>{status.active_project.project_name}</span>
            <span style={{
              padding: '1px 6px', borderRadius: 3, fontSize: 10, fontWeight: 600,
              background: 'rgba(0,213,255,0.1)', color: color.cyan,
            }}>
              {status.active_project.wing}
            </span>
          </div>
        ) : (
          <span style={{ color: color.textDim }}>No project selected</span>
        )}
      </div>

      {/* Source filter pills */}
      <div style={{ padding: '8px 16px', display: 'flex', gap: 4, flexWrap: 'wrap', flexShrink: 0 }}>
        {Object.entries(sourceFilters).map(([key, active]) => (
          <button key={key} onClick={() => toggleFilter(key)} style={{
            padding: '2px 8px', borderRadius: 3, fontSize: 10, fontWeight: 500,
            background: active ? 'rgba(0,213,255,0.08)' : 'rgba(255,255,255,0.02)',
            border: `1px solid ${active ? 'rgba(0,213,255,0.2)' : color.border}`,
            color: active ? color.cyan : color.textDim, cursor: 'pointer',
          }}>
            {key}
          </button>
        ))}
        <span style={{ marginLeft: 'auto', color: color.textDim, fontSize: 10 }}>
          {filteredEvents.length} events
        </span>
      </div>

      {/* Live event tail */}
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto', padding: '0 16px' }}>
        {filteredEvents.slice().reverse().map((event: any, i: number) => {
          const ts = event.timestamp ? new Date(event.timestamp).toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '';
          const surface = event.source_surface ?? '?';
          const type = event.event_type ?? '?';
          const isExpanded = expandedEvent === i;
          return (
            <div key={i} onClick={() => setExpandedEvent(isExpanded ? null : i)} style={{
              padding: '4px 0', borderBottom: `1px solid rgba(255,255,255,0.03)`,
              cursor: 'pointer',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span style={{ color: color.textDim, fontFamily: font.mono, fontSize: 10, width: 56, flexShrink: 0 }}>{ts}</span>
                <span style={{
                  padding: '0 4px', borderRadius: 2, fontSize: 9, fontWeight: 600,
                  background: surfaceColor(surface), color: '#fff',
                }}>
                  {surface.replace(/Picker$/, '')}
                </span>
                <span style={{ color: color.textMuted, fontSize: 11, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {type}
                </span>
              </div>
              {isExpanded && (
                <pre style={{ margin: '4px 0 4px 62px', fontSize: 10, color: color.textDim, fontFamily: font.mono, whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
                  {JSON.stringify(event.payload, null, 2)}
                </pre>
              )}
            </div>
          );
        })}
      </div>

      {/* Recent memories (collapsible) */}
      <div style={{ borderTop: `1px solid ${color.border}`, flexShrink: 0 }}>
        <button onClick={() => setMemoriesOpen(!memoriesOpen)} style={{
          width: '100%', padding: '8px 16px', display: 'flex', alignItems: 'center', gap: 6,
          background: 'transparent', border: 'none', cursor: 'pointer', color: color.textMuted, fontSize: 11, fontWeight: 600,
        }}>
          <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={3}
            style={{ transform: memoriesOpen ? 'rotate(90deg)' : 'rotate(0deg)', transition: `transform 150ms ${ease.out}` }}>
            <path d="M9 18l6-6-6-6" />
          </svg>
          Recent Ambient Memories ({memories.length})
        </button>
        {memoriesOpen && (
          <div style={{ maxHeight: 200, overflow: 'auto', padding: '0 16px 8px' }}>
            {memories.map(m => (
              <div key={m.id} onClick={() => setExpandedMemory(expandedMemory === m.id ? null : m.id)} style={{
                padding: '4px 0', borderBottom: `1px solid rgba(255,255,255,0.03)`, cursor: 'pointer',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  {m.wing && (
                    <span style={{ padding: '0 4px', borderRadius: 2, fontSize: 9, background: 'rgba(0,213,255,0.1)', color: color.cyan }}>
                      {m.wing}
                    </span>
                  )}
                  <span style={{ color: color.textMuted, fontSize: 11, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {m.content.split('\n')[0]}
                  </span>
                </div>
                {expandedMemory === m.id && (
                  <pre style={{ margin: '4px 0 4px 0', fontSize: 10, color: color.textDim, fontFamily: font.mono, whiteSpace: 'pre-wrap' }}>
                    {m.content}
                  </pre>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Current digest (collapsible) */}
      <div style={{ borderTop: `1px solid ${color.border}`, flexShrink: 0 }}>
        <button onClick={() => setDigestOpen(!digestOpen)} style={{
          width: '100%', padding: '8px 16px', display: 'flex', alignItems: 'center', gap: 6,
          background: 'transparent', border: 'none', cursor: 'pointer', color: color.textMuted, fontSize: 11, fontWeight: 600,
        }}>
          <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={3}
            style={{ transform: digestOpen ? 'rotate(90deg)' : 'rotate(0deg)', transition: `transform 150ms ${ease.out}` }}>
            <path d="M9 18l6-6-6-6" />
          </svg>
          Current Digest
        </button>
        {digestOpen && digest && (
          <div style={{ maxHeight: 200, overflow: 'auto', padding: '0 16px 8px', fontSize: 10, color: color.textDim }}>
            <div style={{ marginBottom: 4 }}>
              <strong style={{ color: color.textMuted }}>Live State:</strong>
              <pre style={{ fontFamily: font.mono, whiteSpace: 'pre-wrap', margin: '2px 0' }}>
                {JSON.stringify(digest.live_state, null, 2)}
              </pre>
            </div>
            <div style={{ marginBottom: 4 }}>
              <strong style={{ color: color.textMuted }}>Recent Events:</strong> {digest.recent_events.length}
            </div>
            {digest.probed_memories.length > 0 && (
              <div style={{ marginBottom: 4 }}>
                <strong style={{ color: color.textMuted }}>Probed Memories ({digest.probed_memories.length}):</strong>
                {digest.probed_memories.map((m, i) => (
                  <div key={i} style={{ padding: '2px 0', borderBottom: '1px solid rgba(255,255,255,0.02)' }}>
                    <span style={{ color: color.cyan, fontSize: 9 }}>relevance: {m.relevance?.toFixed(2)}</span>{' '}
                    {m.content?.slice(0, 120)}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function surfaceColor(surface: string): string {
  const s = surface.toLowerCase();
  if (s.includes('chat')) return 'rgba(0,180,255,0.6)';
  if (s.includes('browser')) return 'rgba(180,80,255,0.6)';
  if (s.includes('terminal')) return 'rgba(0,200,100,0.6)';
  if (s.includes('project')) return 'rgba(255,180,0,0.6)';
  if (s.includes('skill')) return 'rgba(255,100,100,0.6)';
  return 'rgba(100,100,100,0.6)';
}
