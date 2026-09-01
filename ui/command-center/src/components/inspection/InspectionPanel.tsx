import { useEffect, useState, useCallback, type CSSProperties } from 'react';
import { FiChevronRight, FiX } from 'react-icons/fi';
import { ease, font, radius, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { api } from '../../lib/api';
import { wireEventType } from '../../lib/wireEvent';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';

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
  const { colors } = useTheme();
  // `glassHi`, not `glass`: this is a full-height 400px fixed rail — an
  // inspector, Apple's largest glass shape after the sidebar — and large glass
  // takes MORE opacity, not less, to hold legibility over whatever screen it
  // is covering. It floats over everything at z-index 10000, so unlike the
  // chat dock it genuinely has a backdrop to sample.
  const glass = useGlass('glassHi');
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

  // The Brain view lives in the MAIN window's workspaces; this panel runs in the
  // separate chat webview (its own store, no loaded workspaces). So we emit a
  // global Tauri event the main window honors, then surface that window.
  const openBrain = async () => {
    if (!('__TAURI_INTERNALS__' in window)) { onClose(); return; }
    try {
      const { emit } = await import('@tauri-apps/api/event');
      await emit('app_navigate', { tool_type: 'memory', reason: 'Opening Brain' });
      const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
      const main = await WebviewWindow.getByLabel('main');
      if (main) { await main.show(); await main.setFocus(); }
    } catch { /* graceful — non-Tauri or window API unavailable */ }
    onClose();
  };

  // Resolves `false` when the call fails so the button cannot confirm a pause
  // that never happened — the catch here swallows its own error.
  const togglePause = async () => {
    try {
      const endpoint = paused ? '/activity/resume' : '/activity/pause';
      await api.fetchAuthed(endpoint, { method: 'POST' });
      setPaused(!paused);
      return true;
    } catch { /* ignore */ return false; }
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
      ...glass,
      borderLeft: `1px solid ${colors.borderHi}`,
      display: 'flex', flexDirection: 'column',
      fontFamily: font.body, fontSize: textSize.caption,
      boxShadow: `${glass.boxShadow}, ${colors.cardShadow}`,
    }}>
      {/* Header */}
      <div style={{
        padding: '12px 16px', borderBottom: `1px solid ${colors.border}`,
        display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0,
      }}>
        <span style={{ flex: 1, fontWeight: 600, fontSize: textSize.small, color: colors.text }}>
          What your agent sees
        </span>
        <Button
          colors={colors}
          onClick={togglePause}
          // The label flips Pause↔Resume; that IS the confirmation, so a tick
          // on top of it would be saying the same thing twice.
          flashSuccess={false}
          style={{
            '--pa-btn-bg': paused ? `${colors.warning}1f` : colors.cyanSoft,
            '--pa-btn-fg': paused ? colors.warning : colors.cyan,
            '--pa-btn-border': paused ? `${colors.warning}4d` : colors.border,
            '--pa-btn-bg-hover': paused ? `${colors.warning}33` : colors.cyanGlow,
            '--pa-btn-border-hover': paused ? colors.warning : colors.borderHi,
            '--pa-btn-bg-active': paused ? `${colors.warning}1f` : colors.cyanSoft,
            '--pa-btn-pad': '4px 10px',
            '--pa-btn-radius': `${radius.xs}px`,
          } as CSSProperties}
        >
          {paused ? 'Resume' : 'Pause'}
        </Button>
        <Button
          colors={colors}
          onClick={openBrain}
          style={{
            '--pa-btn-bg': colors.surfaceHi,
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-border': colors.border,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-pad': '4px 10px',
            '--pa-btn-radius': `${radius.xs}px`,
          } as CSSProperties}
        >
          Open Brain
        </Button>
        <Button
          colors={colors}
          onClick={onClose}
          aria-label="Close"
          style={{
            '--pa-btn-bg': colors.surfaceHi,
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-border': colors.border,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-pad': '0',
            '--pa-btn-radius': `${radius.xs}px`,
            width: 24, height: 24,
          } as CSSProperties}
        >
          <FiX size={10} />
        </Button>
      </div>

      {/* Active project */}
      <div style={{ padding: '8px 16px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0 }}>
        {status?.active_project ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ color: colors.textMuted }}>Project:</span>
            <span style={{ color: colors.text, fontWeight: 500 }}>{status.active_project.project_name}</span>
            <span style={{
              padding: '1px 6px', borderRadius: 3, fontSize: 10, fontWeight: 600,
              background: colors.cyanSoft, color: colors.cyan,
            }}>
              {status.active_project.wing}
            </span>
          </div>
        ) : (
          <span style={{ color: colors.textDim }}>No project selected</span>
        )}
      </div>

      {/* Source filter pills */}
      <div style={{ padding: '8px 16px', display: 'flex', gap: 4, flexWrap: 'wrap', flexShrink: 0 }}>
        {Object.entries(sourceFilters).map(([key, active]) => (
          <Button
            key={key}
            colors={colors}
            onClick={() => toggleFilter(key)}
            style={{
              '--pa-btn-bg': active ? colors.cyanSoft : 'transparent',
              '--pa-btn-fg': active ? colors.cyan : colors.textDim,
              '--pa-btn-border': active ? colors.borderHi : colors.border,
              '--pa-btn-bg-hover': active ? colors.cyanGlow : colors.surfaceHi,
              '--pa-btn-fg-hover': active ? colors.cyan : colors.textMuted,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-bg-active': active ? colors.cyanSoft : colors.surface,
              '--pa-btn-pad': '2px 8px',
              '--pa-btn-radius': '3px',
              fontSize: 10,
            } as CSSProperties}
          >
            {key}
          </Button>
        ))}
        <span style={{ marginLeft: 'auto', color: colors.textDim, fontSize: 10 }}>
          {filteredEvents.length} events
        </span>
      </div>

      {/* Live event tail */}
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto', padding: '0 16px' }}>
        {filteredEvents.slice().reverse().map((event: any, i: number) => {
          const ts = event.timestamp ? new Date(event.timestamp).toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '';
          const surface = event.source_surface ?? '?';
          const type = wireEventType(event) || '?';
          const isExpanded = expandedEvent === i;
          return (
            <div key={i} onClick={() => setExpandedEvent(isExpanded ? null : i)} style={{
              padding: '4px 0', borderBottom: `1px solid ${colors.border}`,
              cursor: 'pointer',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span style={{ color: colors.textDim, fontFamily: font.mono, fontSize: 10, width: 56, flexShrink: 0 }}>{ts}</span>
                <span style={{
                  padding: '0 4px', borderRadius: 2, fontSize: 10, fontWeight: 600,
                  background: surfaceColor(surface, colors), color: colors.textOnAccent,
                }}>
                  {surface.replace(/Picker$/, '')}
                </span>
                <span style={{ color: colors.textMuted, fontSize: textSize.micro, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {type}
                </span>
              </div>
              {isExpanded && (
                <pre style={{ margin: '4px 0 4px 62px', fontSize: 10, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
                  {JSON.stringify(event.payload, null, 2)}
                </pre>
              )}
            </div>
          );
        })}
      </div>

      {/* Recent memories (collapsible) */}
      <div style={{ borderTop: `1px solid ${colors.border}`, flexShrink: 0 }}>
        {/* A disclosure toggle, not an action: nothing is awaited, so the
            pending floor and the success tick would both be wrong signals, and
            the aria-expanded/aria-controls pairing is what actually describes
            it. It takes the shared `.pa-btn` interaction rules directly. */}
        <button
          type="button"
          className="pa-btn"
          aria-expanded={memoriesOpen}
          aria-controls="inspection-memories"
          onClick={() => setMemoriesOpen(!memoriesOpen)}
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-pad': '8px 16px',
            '--pa-btn-radius': '0',
            '--pa-btn-weight': 600,
            display: 'flex', width: '100%', justifyContent: 'flex-start', gap: 6,
            borderWidth: 0, fontSize: textSize.micro,
          } as CSSProperties}
        >
          <FiChevronRight size={8}
            style={{ transform: memoriesOpen ? 'rotate(90deg)' : 'rotate(0deg)', transition: `transform 150ms ${ease.out}` }} />
          Recent Ambient Memories ({memories.length})
        </button>
        {memoriesOpen && (
          <div id="inspection-memories" style={{ maxHeight: 200, overflow: 'auto', padding: '0 16px 8px' }}>
            {memories.map(m => (
              <div key={m.id} onClick={() => setExpandedMemory(expandedMemory === m.id ? null : m.id)} style={{
                padding: '4px 0', borderBottom: `1px solid ${colors.border}`, cursor: 'pointer',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  {m.wing && (
                    <span style={{ padding: '0 4px', borderRadius: 2, fontSize: 10, background: colors.cyanSoft, color: colors.cyan }}>
                      {m.wing}
                    </span>
                  )}
                  <span style={{ color: colors.textMuted, fontSize: textSize.micro, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {m.content.split('\n')[0]}
                  </span>
                </div>
                {expandedMemory === m.id && (
                  <pre style={{ margin: '4px 0 4px 0', fontSize: 10, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'pre-wrap' }}>
                    {m.content}
                  </pre>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Current digest (collapsible) */}
      <div style={{ borderTop: `1px solid ${colors.border}`, flexShrink: 0 }}>
        {/* Disclosure toggle — same reasoning as the memories header above. */}
        <button
          type="button"
          className="pa-btn"
          aria-expanded={digestOpen}
          aria-controls="inspection-digest"
          onClick={() => setDigestOpen(!digestOpen)}
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-pad': '8px 16px',
            '--pa-btn-radius': '0',
            '--pa-btn-weight': 600,
            display: 'flex', width: '100%', justifyContent: 'flex-start', gap: 6,
            borderWidth: 0, fontSize: textSize.micro,
          } as CSSProperties}
        >
          <FiChevronRight size={8}
            style={{ transform: digestOpen ? 'rotate(90deg)' : 'rotate(0deg)', transition: `transform 150ms ${ease.out}` }} />
          Current Digest
        </button>
        {digestOpen && digest && (
          <div id="inspection-digest" style={{ maxHeight: 200, overflow: 'auto', padding: '0 16px 8px', fontSize: 10, color: colors.textDim }}>
            <div style={{ marginBottom: 4 }}>
              <strong style={{ color: colors.textMuted }}>Live State:</strong>
              <pre style={{ fontFamily: font.mono, whiteSpace: 'pre-wrap', margin: '2px 0' }}>
                {JSON.stringify(digest.live_state, null, 2)}
              </pre>
            </div>
            <div style={{ marginBottom: 4 }}>
              <strong style={{ color: colors.textMuted }}>Recent Events:</strong> {digest.recent_events.length}
            </div>
            {digest.probed_memories.length > 0 && (
              <div style={{ marginBottom: 4 }}>
                <strong style={{ color: colors.textMuted }}>Probed Memories ({digest.probed_memories.length}):</strong>
                {digest.probed_memories.map((m, i) => (
                  <div key={i} style={{ padding: '2px 0', borderBottom: `1px solid ${colors.border}` }}>
                    <span style={{ color: colors.cyan, fontSize: 10 }}>relevance: {m.relevance?.toFixed(2)}</span>{' '}
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

function surfaceColor(surface: string, colors: { cyan: string; purple: string; success: string; warning: string; danger: string; textDim: string }): string {
  const s = surface.toLowerCase();
  if (s.includes('chat')) return colors.cyan;
  if (s.includes('browser')) return colors.purple;
  if (s.includes('terminal')) return colors.success;
  if (s.includes('project')) return colors.warning;
  if (s.includes('skill')) return colors.danger;
  return colors.textDim;
}
