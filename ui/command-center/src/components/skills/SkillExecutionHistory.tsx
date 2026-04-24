import { useEffect, useState } from 'react';
import { FiCheckCircle, FiXCircle, FiClock } from 'react-icons/fi';
import { api } from '../../lib/api';

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
  const [executions, setExecutions] = useState<Execution[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    api.getSkillExecutions(skillId).then(data => {
      setExecutions(data);
      setLoading(false);
    });
  }, [skillId]);

  if (loading) {
    return (
      <div>
        <label className="block text-[10px] font-mono uppercase text-dark-muted mb-2">Execution History</label>
        <div className="text-[10px] font-mono text-dark-muted/60">Loading...</div>
      </div>
    );
  }

  return (
    <div>
      <label className="block text-[10px] font-mono uppercase text-dark-muted mb-2">
        Execution History {executions.length > 0 && `(${executions.length})`}
      </label>
      {executions.length === 0 ? (
        <div className="text-[10px] font-mono text-dark-muted/60">No executions yet.</div>
      ) : (
        <div className="space-y-1.5">
          {executions.map(exec => {
            const duration = exec.completed_at
              ? Math.round((new Date(exec.completed_at).getTime() - new Date(exec.started_at).getTime()) / 1000)
              : null;

            return (
              <div key={exec.id} className="flex items-start gap-2 rounded bg-dark-surface/50 p-2">
                {exec.status === 'completed' ? (
                  <FiCheckCircle size={12} className="text-emerald-400 shrink-0 mt-0.5" />
                ) : exec.status === 'failed' ? (
                  <FiXCircle size={12} className="text-red-400 shrink-0 mt-0.5" />
                ) : (
                  <FiClock size={12} className="text-amber-400 shrink-0 mt-0.5" />
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 text-[10px] font-mono">
                    <span className="text-dark-text/80">
                      {new Date(exec.started_at).toLocaleString()}
                    </span>
                    {duration !== null && (
                      <span className="text-dark-muted">{duration}s</span>
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
