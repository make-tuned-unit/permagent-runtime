/**
 * Settings → Agents → Skills — the Skills Library's front door.
 *
 * The Library is a whole overlay-class feature that, until this section
 * existed, had exactly two UI-initiated entry points: accepting a skill
 * proposal, and clicking a skill inside Automate's condensed "Learned" list.
 * Neither is a way to go *look* at what the agent has learned, so in practice
 * the surface was reachable only by accident.
 *
 * Placement is the ruled one (J4): skills are what the agent has learned, so
 * they belong beside the agent roster rather than as a tenth sidebar workspace.
 * The section states the count it is opening rather than a bare button, so the
 * door says what is behind it — and an empty library says how skills get
 * created instead of reading as a broken fetch.
 */

import { useEffect } from 'react';
import { Section } from '../atoms';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';
import { useCommandCenter } from '../../../lib/store';
import { type SkillState } from '../../../lib/store';

export function SkillsSection() {
  const { colors } = useTheme();
  // Defaulted at the read: this section is rendered inside a pane whose own
  // tests mock the store down to the roster keys, and a front door that throws
  // is worse than no front door.
  const skills = useCommandCenter(s => s.skills) as SkillState[] | undefined;
  const skillsLoading = useCommandCenter(s => s.skillsLoading) as boolean | undefined;
  const skillsError = useCommandCenter(s => s.skillsError) as string | null | undefined;
  const loadSkills = useCommandCenter(s => s.loadSkills) as (() => void) | undefined;
  const setActivePanel = useCommandCenter(s => s.setActivePanel) as
    | ((p: 'skills') => void)
    | undefined;

  useEffect(() => { loadSkills?.(); }, [loadSkills]);

  const list = skills ?? [];
  const count = list.length;

  return (
    <Section
      title="Skills"
      sub="Procedures the agent has learned from your work and can re-run without being re-taught. The Library is where you read, edit and retire them."
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 14, flexWrap: 'wrap' }}>
        <Button
          colors={colors}
          type="button"
          data-testid="open-skills-library"
          flashSuccess={false}
          onClick={() => setActivePanel?.('skills')}
        >
          Open Skills Library
        </Button>
        <span
          style={{
            fontSize: 12,
            color: skillsError ? colors.warning : colors.textMuted,
            lineHeight: 1.5,
          }}
          data-testid="skills-count"
        >
          {skillsError
            // Not "0 skills": a count that could not be read is not a count.
            ? `Couldn't count them — ${skillsError}`
            : skillsLoading && count === 0
              ? 'Counting what has been learned…'
              : count === 0
                ? 'Nothing learned yet — skills are proposed after a task the agent could repeat, and saved from the chat banner.'
                : `${count} learned skill${count === 1 ? '' : 's'}.`}
        </span>
      </div>
    </Section>
  );
}
