import { useEffect, useRef, useState } from 'react';
import { FiCheckCircle, FiXCircle, FiClock } from 'react-icons/fi';
import { api } from '../../lib/api';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Execution {
  id: string;
  status: string;
  started_at: string;
  completed_at?: string;
  error_message?: string;
}

interface SkillExecutionHistoryProps {
  skillId: string;
}

export function SkillExecutionHistory({ skillId }: SkillExecutionHistoryProps) {
  const { colors } = useTheme();
  const [executions, setExecutions] = useState<Execution[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Monotonic request sequence (the useBrainData seqRef / TimelineCard
  // fetchSeq pattern): a response only lands if no newer skill was selected
  // since, so a slow stale reply can never clobber the current skill's history.
  const fetchSeq = useRef(0);

  useEffect(() => {
    const seq = ++fetchSeq.current;
    setLoading(true);
    setError(null);
    api.getSkillExecutions(skillId)
      .then(data => {
        if (seq !== fetchSeq.current) return; // skill changed mid-flight
        setExecutions(data);
        setLoading(false);
      })
      .catch(err => {
        if (seq !== fetchSeq.current) return;
        // A backend failure is an ERROR, not an empty history — the old
        // `.catch(() => [])` rendered daemon outages as "No executions yet".
        setExecutions([]);
        setError(err instanceof Error ? err.message : 'Could not load executions');
        setLoading(false);
      });
  }, [skillId]);

  if (loading) {
    return (
      <div>
        <label
          className="block text-[10px] uppercase mb-2"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Execution History
        </label>
        <div className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted, opacity: 0.8 }}>Loading...</div>
      </div>
    );
  }

  return (
    <div>
      <label
        className="block text-[10px] uppercase mb-2"
        style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
      >
        Execution History {executions.length > 0 && `(${executions.length})`}
      </label>
      {error ? (
        <div className="text-[10px]" style={{ fontFamily: font.mono, color: colors.danger }}>
          Couldn't load execution history — {error}
        </div>
      ) : executions.length === 0 ? (
        <div className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted, opacity: 0.8 }}>No executions yet.</div>
      ) : (
        <div className="space-y-1.5">
          {executions.map(exec => {
            const duration = exec.completed_at
              ? Math.round((new Date(exec.completed_at).getTime() - new Date(exec.started_at).getTime()) / 1000)
              : null;

            return (
              <div key={exec.id} className="flex items-start gap-2 rounded p-2" style={{ backgroundColor: `${colors.surface}80` }}>
                {exec.status === 'completed' ? (
                  <FiCheckCircle size={12} className="text-emerald-400 shrink-0 mt-0.5" />
                ) : exec.status === 'failed' ? (
                  <FiXCircle size={12} className="text-red-400 shrink-0 mt-0.5" />
                ) : (
                  <FiClock size={12} className="text-amber-400 shrink-0 mt-0.5" />
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 text-[10px]" style={{ fontFamily: font.mono }}>
                    <span style={{ color: colors.text, opacity: 0.8 }}>
                      {new Date(exec.started_at).toLocaleString()}
                    </span>
                    {duration !== null && (
                      <span style={{ color: colors.textMuted }}>{duration}s</span>
                    )}
                    <span className={`uppercase ${
                      exec.status === 'completed' ? 'text-emerald-400' :
                      exec.status === 'failed' ? 'text-red-400' : 'text-amber-400'
                    }`}>
                      {exec.status}
                    </span>
                  </div>
                  {exec.error_message && (
                    <p className="text-[10px] text-red-400/80 mt-0.5 truncate">{exec.error_message}</p>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
