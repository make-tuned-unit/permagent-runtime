import { useEffect, useState, useCallback } from 'react';
import { api } from '../../lib/api';
import { font, ease, space } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Props {
  visible: boolean;
}

export function PreTurnPreview({ visible }: Props) {
  const { colors } = useTheme();
  const [label, setLabel] = useState<string | null>(null);

  const fetchPreview = useCallback(async () => {
    try {
      const [digestRes, statusRes] = await Promise.all([
        api.fetchAuthed('/activity/current-digest'),
        api.fetchAuthed('/activity/ingest-status'),
      ]);
      if (!digestRes.ok || !statusRes.ok) { setLabel(null); return; }

      const digest = await digestRes.json();
      const status = await statusRes.json();

      const events = digest.recent_events ?? [];
      const memories = digest.probed_memories ?? [];
      if (events.length === 0 && memories.length === 0) { setLabel(null); return; }

      const project = status.active_project?.project_name;
      const parts: string[] = ['The agent will consider'];
      if (project) parts.push(`your last 10 minutes in ${project}`);
      else parts.push('your last 10 minutes');
      parts.push('—');
      const detail: string[] = [];
      if (events.length > 0) detail.push(`${events.length} events`);
      if (memories.length > 0) detail.push(`${memories.length} memories from today`);
      setLabel(parts.join(' ') + ' ' + detail.join(', '));
    } catch {
      setLabel(null);
    }
  }, []);

  useEffect(() => {
    if (visible) fetchPreview();
  }, [visible, fetchPreview]);

  if (!visible || !label) return null;

  return (
    <div style={{
      padding: `${space.xxs}px ${space.xl}px ${space.xs}px`,
      fontSize: 10,
      fontFamily: font.body,
      fontStyle: 'italic',
      color: colors.textDim,
      opacity: 0.8,
      transition: `opacity 200ms ${ease.out}`,
      overflow: 'hidden',
      textOverflow: 'ellipsis',
      whiteSpace: 'nowrap',
    }}>
      {label}
    </div>
  );
}
