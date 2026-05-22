import { FiZap, FiClock } from 'react-icons/fi';
import type { SkillState } from '../../lib/store';

interface SkillCardProps {
  skill: SkillState;
  isSelected: boolean;
  onSelect: (id: string) => void;
}

export function SkillCard({ skill, isSelected, onSelect }: SkillCardProps) {
  const isActive = skill.status === 'active';

  return (
    <button
      onClick={() => onSelect(skill.id)}
      className={`w-full text-left rounded-lg border p-4 transition cursor-pointer ${
        isSelected
          ? 'border-accent/50 bg-accent/5'
          : 'border-dark-border bg-[#0D1424] hover:border-accent/30'
      }`}
    >
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${
              isActive ? 'bg-emerald-400' : 'bg-slate-500'
            }`} />
            <h3 className="font-medium text-sm text-dark-text truncate">{skill.name}</h3>
          </div>
          {skill.description && (
            <p className="text-xs text-dark-muted mb-2 line-clamp-2 pl-3.5">{skill.description}</p>
          )}
          <div className="flex items-center gap-3 text-[10px] text-dark-muted font-mono pl-3.5">
            {skill.trigger_type && (
              <span className="flex items-center gap-1">
                <FiZap size={9} />
                {skill.trigger_type}
              </span>
            )}
            {skill.usageCount !== undefined && (
              <span>{skill.usageCount} runs</span>
            )}
            {skill.last_run && (
              <span className="flex items-center gap-1">
                <FiClock size={9} />
                {new Date(skill.last_run).toLocaleDateString()}
              </span>
            )}
          </div>
        </div>
        {skill.status && (
          <span className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] font-mono uppercase ${
            skill.status === 'active' ? 'bg-emerald-400/10 text-emerald-400' :
            skill.status === 'paused' ? 'bg-amber-400/10 text-amber-400' :
            'bg-slate-400/10 text-slate-400'
          }`}>
            {skill.status}
          </span>
        )}
      </div>
    </button>
  );
}
