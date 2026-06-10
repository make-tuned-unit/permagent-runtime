import { FiZap, FiX } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function SkillPromptBanner() {
  const { colors } = useTheme();
  const proposal = useCommandCenter(s => s.pendingSkillProposal);
  const saveSkillProposal = useCommandCenter(s => s.saveSkillProposal);
  const dismissSkillProposal = useCommandCenter(s => s.dismissSkillProposal);

  if (!proposal) return null;

  return (
    <div
      className="mx-4 mb-2 rounded-lg px-4 py-3"
      style={{ border: `1px solid ${colors.cyan}4D`, backgroundColor: `${colors.cyan}0D` }}
    >
      <div className="flex items-start gap-3">
        <FiZap size={16} className="shrink-0 mt-0.5" style={{ color: colors.cyan }} />
        <div className="flex-1 min-w-0">
          <div className="text-[12px] mb-1" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>
            Repeated pattern detected
          </div>
          <div className="text-[11px] mb-2 line-clamp-2" style={{ fontFamily: font.body, color: colors.textMuted }}>
            {proposal.description}
          </div>
          <div className="text-[10px] mb-2" style={{ fontFamily: font.mono, color: colors.textDim }}>
            Seen {proposal.occurrence_count} time{proposal.occurrence_count !== 1 ? 's' : ''}
            {proposal.tool_used && <> using <span style={{ color: colors.cyan }}>{proposal.tool_used}</span></>}
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={saveSkillProposal}
              className="rounded-md px-3 py-1 text-[11px] transition"
              style={{ fontFamily: font.mono, backgroundColor: `${colors.cyan}33`, color: colors.cyan }}
            >
              Save as Skill
            </button>
            <button
              onClick={dismissSkillProposal}
              className="rounded-md px-2 py-1 text-[11px] transition"
              style={{ fontFamily: font.mono, color: colors.textMuted }}
            >
              <FiX size={12} className="inline mr-0.5" />
              Dismiss
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
