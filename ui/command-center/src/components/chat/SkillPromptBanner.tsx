import { FiZap, FiX } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';

export function SkillPromptBanner() {
  const proposal = useCommandCenter(s => s.pendingSkillProposal);
  const saveSkillProposal = useCommandCenter(s => s.saveSkillProposal);
  const dismissSkillProposal = useCommandCenter(s => s.dismissSkillProposal);

  if (!proposal) return null;

  return (
    <div className="mx-4 mb-2 rounded-lg border border-accent/30 bg-accent/5 px-4 py-3">
      <div className="flex items-start gap-3">
        <FiZap size={16} className="text-accent shrink-0 mt-0.5" />
        <div className="flex-1 min-w-0">
          <div className="text-[12px] font-mono text-dark-text mb-1">
            Repeated pattern detected
          </div>
          <div className="text-[11px] font-mono text-dark-muted mb-2 line-clamp-2">
            {proposal.description}
          </div>
          <div className="text-[10px] font-mono text-dark-muted/60 mb-2">
            Seen {proposal.occurrence_count} time{proposal.occurrence_count !== 1 ? 's' : ''}
            {proposal.tool_used && <> using <span className="text-accent/70">{proposal.tool_used}</span></>}
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={saveSkillProposal}
              className="rounded-md bg-accent/20 px-3 py-1 text-[11px] font-mono text-accent hover:bg-accent/30 transition"
            >
              Save as Skill
            </button>
            <button
              onClick={dismissSkillProposal}
              className="rounded-md px-2 py-1 text-[11px] font-mono text-dark-muted hover:text-dark-text hover:bg-white/5 transition"
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
