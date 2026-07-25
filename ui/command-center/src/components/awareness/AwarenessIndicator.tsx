import { useEffect, useRef, useState, useCallback } from 'react';
import { api } from '../../lib/api';
import { relativeTimeAgo } from '../../lib/time-decay';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface DigestSummary {
  project_name: string | null;
  event_count: number;
  memory_count: number;
  last_event_time: string | null;
}

interface Props {
  onOpenInspection: () => void;
}

export function AwarenessIndicator({ onOpenInspection }: Props) {
  const { colors } = useTheme();
  const [summary, setSummary] = useState<DigestSummary | null>(null);
  const requestGeneration = useRef(0);

  const fetchDigest = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const [digestRes, statusRes] = await Promise.all([
        api.fetchAuthed('/activity/current-digest'),
        api.fetchAuthed('/activity/ingest-status'),
      ]);
      if (!digestRes.ok || !statusRes.ok || generation !== requestGeneration.current) return;

      const digest = await digestRes.json();
      const status = await statusRes.json();
      if (generation !== requestGeneration.current) return;

      if (!digest || typeof digest !== 'object' || !status || typeof status !== 'object') {
        throw new Error('Invalid awareness response');
      }
      const events = Array.isArray(digest.recent_events) ? digest.recent_events : [];
      const lastTs = events.length > 0 ? events[events.length - 1]?.timestamp : null;

      setSummary({
        project_name: status.active_project?.project_name ?? null,
        event_count: events.length,
        memory_count: Array.isArray(digest.probed_memories) ? digest.probed_memories.length : 0,
        last_event_time: lastTs,
      });
    } catch {
      if (generation === requestGeneration.current) setSummary(null);
    }
  }, []);

  useEffect(() => {
    fetchDigest();
    const interval = setInterval(fetchDigest, 5000);
    return () => clearInterval(interval);
  }, [fetchDigest]);

  if (!summary) return null;

  const { project_name, event_count, memory_count, last_event_time } = summary;

  if (event_count === 0 && !project_name) {
    return (
      <div onClick={onOpenInspection} style={containerStyle}>
        <span style={dotStyle('#64748b')} />
        <span style={textStyle(colors)}>No recent activity</span>
      </div>
    );
  }

  const stale = last_event_time ? relativeTimeAgo(last_event_time) : '';
  const parts: string[] = [];
  if (project_name) parts.push(`Aware of ${project_name}`);
  if (event_count > 0) parts.push(`${event_count} recent events`);
  if (memory_count > 0) parts.push(`${memory_count} relevant memories`);
  if (stale) parts.push(`last action ${stale}`);

  const label = parts.join(' · ');
  const isActive = event_count > 0 && !stale;

  return (
    <div onClick={onOpenInspection} style={containerStyle}>
      <span style={dotStyle(isActive ? colors.cyan : '#64748b')} />
      <span style={textStyle(colors)}>{label}</span>
    </div>
  );
}

const containerStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 6,
  padding: '4px 12px',
  cursor: 'pointer',
  userSelect: 'none',
  borderTop: `1px solid rgba(255,255,255,0.04)`,
};

const dotStyle = (bg: string): React.CSSProperties => ({
  width: 5,
  height: 5,
  borderRadius: '50%',
  background: bg,
  flexShrink: 0,
});

const textStyle = (colors: { textDim: string }): React.CSSProperties => ({
  fontSize: 10,
  fontFamily: font.body,
  color: colors.textDim,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
});
