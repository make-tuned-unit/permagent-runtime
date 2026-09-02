import { useState } from 'react';
import { FiZap, FiX } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export function SkillPromptBanner() {
  const { colors } = useTheme();
  const proposal = useCommandCenter(s => s.pendingSkillProposal);
  const saveSkillProposal = useCommandCenter(s => s.saveSkillProposal);
  const dismissSkillProposal = useCommandCenter(s => s.dismissSkillProposal);
  const [failed, setFailed] = useState(false);

  // Save-as-Skill is one of only two ways a skill is ever created, so a
  // failure has to be visible here rather than in the console.
  const save = async () => {
    setFailed(false);
    const ok = await saveSkillProposal();
    if (!ok) setFailed(true);
    return ok;
  };

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
            <Button
              colors={colors}
              variant="ghostOn"
              type="button"
              onClick={save}
              style={{ fontFamily: font.mono }}
            >
              Save as Skill
            </Button>
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={dismissSkillProposal}
              style={{ fontFamily: font.mono, color: colors.textMuted }}
            >
              <FiX size={12} className="inline mr-0.5" />
              Dismiss
            </Button>
          </div>
          {failed && (
            <div
              role="alert"
              className="text-[11px] mt-2"
              style={{ fontFamily: font.body, color: colors.danger }}
            >
              Couldn't save this as a skill — the daemon rejected it. Try again.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
