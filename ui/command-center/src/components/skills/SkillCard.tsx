import { FiZap, FiClock } from 'react-icons/fi';
import type { SkillState } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface SkillCardProps {
  skill: SkillState;
  isSelected: boolean;
  onSelect: (id: string) => void;
}

export function SkillCard({ skill, isSelected, onSelect }: SkillCardProps) {
  const { colors } = useTheme();
  const isActive = skill.status === 'active';

  return (
    <button
      onClick={() => onSelect(skill.id)}
      className="w-full text-left rounded-lg p-4 transition cursor-pointer"
      style={{
        border: `1px solid ${isSelected ? `${colors.cyan}80` : colors.border}`,
        backgroundColor: isSelected ? `${colors.cyan}0D` : colors.surface,
        boxShadow: colors.cardShadow,
      }}
      onMouseEnter={e => { if (!isSelected) e.currentTarget.style.borderColor = `${colors.cyan}4D`; }}
      onMouseLeave={e => { if (!isSelected) e.currentTarget.style.borderColor = colors.border; }}
    >
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${
              isActive ? 'bg-emerald-400' : 'bg-slate-500'
            }`} />
            <h3 className="text-sm truncate" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>{skill.name}</h3>
          </div>
          {skill.description && (
            <p className="text-xs mb-2 line-clamp-2 pl-3.5" style={{ fontFamily: font.body, color: colors.textMuted }}>{skill.description}</p>
          )}
          <div className="flex items-center gap-3 text-[10px] pl-3.5" style={{ fontFamily: font.mono, color: colors.textMuted }}>
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
          <span style={{ fontFamily: font.mono }} className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] uppercase ${
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
