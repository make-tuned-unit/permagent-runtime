/**
 * The Automate view's "Learned" section body: pending proposals, then saved
 * skills grouped by tool as one compact list.
 *
 * It used to be one card per skill — 37 cards titled with raw call
 * signatures ("copy_to_clipboard:-te…", "orchestrator__goal_s…"), bodies like
 * "orchestrator__goal_status: card_id=[REDACTED]", and a MASTERED/TRUSTED chip
 * read off a number that counts repetition, not competence (see
 * `skillHumanize.matchesLabel`). Now each tool is one row with a readable
 * name and counts; opening a row lists its patterns, and a pattern opens the
 * skill (with its full raw pattern) in the Skills library.
 */
import { useState } from 'react';
import type { CSSProperties } from 'react';
import { FiChevronRight } from 'react-icons/fi';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Chip } from '../common/Chip';
import { Tooltip } from '../common/Tooltip';
import type { SkillProposal, SkillState } from '../../lib/store';
import { withAlpha } from '../dashboard/decisions/format';
import { groupSkills, humanizeSkill, matchesLabel, MATCHES_HINT } from '../skills/skillHumanize';

/** Groups shown before "Show all" — a screenful, not the whole library. */
export const LEARNED_GROUPS_VISIBLE = 8;

function rowVars(colors: ThemeColors): CSSProperties {
  return {
    '--pa-btn-fg': colors.text,
    '--pa-btn-bg-hover': colors.fillHover,
    '--pa-btn-bg-active': colors.fillActive,
    '--pa-btn-pad': `${space.md}px ${space.xl}px`,
    '--pa-btn-radius': `${radius.sm}px`,
    fontFamily: font.body,
    gap: space.lg,
    width: '100%',
    justifyContent: 'flex-start',
    textAlign: 'left',
  } as CSSProperties;
}

export function LearnedList({ skills, proposals, onOpenSkill, onSaveProposal, onDismissProposal, proposalError }: {
  skills: SkillState[];
  proposals: SkillProposal[];
  onOpenSkill: (id: string) => void;
  onSaveProposal: (p: SkillProposal) => Promise<boolean>;
  onDismissProposal: (hash: string) => void;
  proposalError: string | null;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [showAll, setShowAll] = useState(false);
  const groups = groupSkills(skills);
  const visible = showAll ? groups : groups.slice(0, LEARNED_GROUPS_VISIBLE);
  const toggle = (key: string) => setOpen(prev => {
    const next = new Set(prev);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  const count: CSSProperties = { fontSize: 10, color: colors.textDim, fontFamily: font.mono, flexShrink: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
      {proposals.length > 0 && (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: space.xl }}>
          {proposals.map(proposal => {
            const human = humanizeSkill({ id: proposal.argument_shape_hash, name: proposal.description, description: proposal.description, toolUsed: proposal.tool_used });
            return (
              <div key={proposal.argument_shape_hash} data-testid="learned-proposal" style={{
                padding: `${space.xxl}px ${space.xxxl}px`, borderRadius: radius.lg,
                background: withAlpha(colors.warning, 0.04), border: `1px solid ${withAlpha(colors.warning, 0.2)}`,
              }}>
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: space.lg, marginBottom: space.sm }}>
                  <div style={{ width: 8, height: 8, borderRadius: '50%', background: colors.warning, flexShrink: 0, marginTop: space.sm }} />
                  <div style={{ fontSize: textSize.body, fontWeight: 600, fontFamily: font.display, flex: 1, minWidth: 0, lineHeight: 1.35, overflowWrap: 'anywhere' }}>{human.title}</div>
                  <Chip kind="static" tone="warning" style={{ flexShrink: 0 }}>PROPOSED</Chip>
                </div>
                <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5, marginBottom: space.md }}>
                  Seen {proposal.occurrence_count} time{proposal.occurrence_count !== 1 ? 's' : ''}
                  {human.raw && <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, overflowWrap: 'anywhere' }}>{human.raw}</div>}
                </div>
                <div style={{ display: 'flex', gap: space.sm, alignItems: 'center', flexWrap: 'wrap' }}>
                  <Button colors={colors} variant="primary" onClick={() => onSaveProposal(proposal)} style={{ '--pa-btn-pad': '6px 16px', fontFamily: font.body } as CSSProperties}>Save</Button>
                  <Button
                    colors={colors}
                    onClick={() => onDismissProposal(proposal.argument_shape_hash)}
                    style={{
                      '--pa-btn-pad': '4px 10px', '--pa-btn-radius': `${radius.sm}px`, fontFamily: font.body,
                      '--pa-btn-bg': colors.border, '--pa-btn-fg': colors.textMuted, '--pa-btn-border': colors.border,
                      '--pa-btn-bg-hover': colors.surfaceHi, '--pa-btn-fg-hover': colors.text, '--pa-btn-border-hover': colors.borderHi,
                    } as CSSProperties}
                  >Dismiss</Button>
                  {proposalError === proposal.argument_shape_hash && (
                    <span style={{ fontSize: textSize.micro, color: colors.danger }}>
                      Couldn't save this skill. The daemon may be down.
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {groups.length > 0 && (
        <div data-testid="learned-list" style={{ borderRadius: radius.lg, background: colors.surface, border: `1px solid ${colors.border}`, padding: space.xs }}>
          {visible.map(group => {
            const single = group.items.length === 1 ? group.items[0] : null;
            const isOpen = open.has(group.key);
            const archived = group.items.every(i => i.skill.status && i.skill.status !== 'active');
            const head = (
              <>
                {single
                  ? <span style={{ width: 12, flexShrink: 0 }} />
                  : <FiChevronRight size={12} style={{ flexShrink: 0, color: colors.textDim, transform: isOpen ? 'rotate(90deg)' : 'none' }} />}
                <span data-testid="learned-group-label" style={{ fontSize: textSize.small, fontWeight: 600, flex: 1, minWidth: 0, overflowWrap: 'anywhere', color: archived ? colors.textMuted : colors.text }}>
                  {single ? single.human.title : group.label}
                </span>
                {!single && <span style={count}>{group.items.length} patterns</span>}
                {archived && <span style={count}>archived</span>}
                <Tooltip content={MATCHES_HINT}>
                  <span tabIndex={0} style={{ ...count, outline: 'none' }}>{matchesLabel(group.matches)}</span>
                </Tooltip>
              </>
            );
            return (
              <div key={group.key} data-testid="learned-group">
                {single ? (
                  <button
                    type="button"
                    className="pa-btn pa-btn--composite"
                    aria-label={`Open ${single.human.title} in the skills library`}
                    onClick={() => onOpenSkill(single.skill.id)}
                    style={rowVars(colors)}
                  >{head}</button>
                ) : (
                  <>
                    <button
                      type="button"
                      className="pa-btn pa-btn--composite"
                      aria-expanded={isOpen}
                      onClick={() => toggle(group.key)}
                      style={rowVars(colors)}
                    >{head}</button>
                    {isOpen && (
                      <div style={{ paddingLeft: space.huge, paddingBottom: space.xs }}>
                        {group.items.map(({ skill, human }, i) => (
                          <button
                            key={skill.id}
                            type="button"
                            className="pa-btn pa-btn--composite"
                            data-testid="learned-pattern"
                            aria-label={`Open ${human.title} in the skills library`}
                            onClick={() => onOpenSkill(skill.id)}
                            style={{ ...rowVars(colors), '--pa-btn-pad': `${space.sm}px ${space.xl}px` } as CSSProperties}
                          >
                            <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: space.xxs }}>
                              <span style={{ fontSize: textSize.caption, color: colors.text, overflowWrap: 'anywhere' }}>
                                {human.variant ?? `Pattern ${i + 1}`}
                              </span>
                              {human.raw && (
                                <span style={{ fontSize: 10, fontFamily: font.mono, color: colors.textDim, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{human.raw}</span>
                              )}
                            </span>
                            <span style={count}>{matchesLabel(skill.usageCount ?? 0)}</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </>
                )}
              </div>
            );
          })}
          {groups.length > LEARNED_GROUPS_VISIBLE && (
            <Button
              colors={colors}
              variant="bare"
              aria-expanded={showAll}
              onClick={() => setShowAll(v => !v)}
              style={{
                '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.text, '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': `${space.sm}px ${space.xl}px`, fontFamily: font.body, fontSize: textSize.caption,
              } as CSSProperties}
            >{showAll ? 'Show fewer' : `Show all ${groups.length} groups`}</Button>
          )}
        </div>
      )}
    </div>
  );
}
