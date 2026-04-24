import { useEffect } from 'react';
import { FiTrash2, FiLoader, FiZap } from 'react-icons/fi';
import { useCommandCenter, type SkillState } from '../../lib/store';

function SkillCard({ skill, onDelete }: { skill: SkillState; onDelete: (id: string) => void }) {
  return (
    <div className="rounded-lg border border-dark-border bg-[#0D1424] p-4 hover:border-accent/30 transition">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <h3 className="font-medium text-sm text-dark-text truncate">{skill.name}</h3>
            {skill.status && (
              <span className={`rounded px-1.5 py-0.5 text-[9px] font-mono uppercase ${
                skill.status === 'active' ? 'bg-emerald-400/10 text-emerald-400' :
                skill.status === 'draft' ? 'bg-amber-400/10 text-amber-400' :
                'bg-slate-400/10 text-slate-400'
              }`}>
                {skill.status}
              </span>
            )}
          </div>
          <p className="text-xs text-dark-muted mb-2 line-clamp-2">{skill.description}</p>
          <div className="flex items-center gap-3 text-[10px] text-dark-muted font-mono">
            {skill.trigger_type && (
              <span>trigger: {skill.trigger_type}</span>
            )}
            {skill.usage_count !== undefined && (
              <span>{skill.usage_count} runs</span>
            )}
            {skill.last_run && (
              <span>last: {new Date(skill.last_run).toLocaleDateString()}</span>
            )}
          </div>
        </div>
        <button
          onClick={() => onDelete(skill.id)}
          className="shrink-0 rounded p-1.5 text-dark-muted hover:text-red-400 hover:bg-red-400/10 transition"
          title="Delete skill"
        >
          <FiTrash2 size={14} />
        </button>
      </div>
    </div>
  );
}

export function SkillsPanel() {
  const skills = useCommandCenter(s => s.skills);
  const skillsLoading = useCommandCenter(s => s.skillsLoading);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const deleteSkill = useCommandCenter(s => s.deleteSkill);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  return (
    <div className="flex h-full flex-col bg-[#0A0E17]">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
        <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">Skills Library</span>
        <span className="rounded bg-dark-surface px-1.5 py-0.5 text-[10px] font-mono text-dark-muted">
          {skills.length}
        </span>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4">
        {skillsLoading ? (
          <div className="flex items-center justify-center h-32 text-dark-muted">
            <FiLoader size={16} className="animate-spin mr-2" />
            <span className="text-xs font-mono">Loading skills...</span>
          </div>
        ) : skills.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-dark-muted text-xs text-center gap-2 font-mono">
            <FiZap size={24} className="opacity-30 text-accent/40" />
            <div>No skills saved yet.</div>
            <div className="text-[10px]">Skills are created when your agent detects repeated patterns.</div>
          </div>
        ) : (
          <div className="space-y-3">
            {skills.map(skill => (
              <SkillCard key={skill.id} skill={skill} onDelete={deleteSkill} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
