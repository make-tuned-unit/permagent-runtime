import { useState, type CSSProperties } from 'react';
import { FiCheck, FiX } from 'react-icons/fi';
import { useCommandCenter, type SkillState } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

interface SkillEditorProps {
  skill: SkillState;
  onClose: () => void;
}

export function SkillEditor({ skill, onClose }: SkillEditorProps) {
  const { colors } = useTheme();
  const updateSkill = useCommandCenter(s => s.updateSkill);
  const [name, setName] = useState(skill.name);
  const [description, setDescription] = useState(skill.description || '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const hasChanges = name !== skill.name || description !== (skill.description || '');

  // Returns the outcome so the Button primitive never ticks over a save that
  // did not land — `updateSkill` swallows its own error into `ok`.
  const handleSave = async () => {
    if (!hasChanges || !name.trim()) return false;
    setSaving(true);
    setError(null);
    const ok = await updateSkill(skill.id, { name: name.trim(), description: description.trim() });
    setSaving(false);
    // Only close if it actually persisted; otherwise keep the edits and tell the user.
    if (ok) {
      onClose();
    } else {
      setError('Could not save. Please try again.');
    }
    return ok;
  };

  const inputStyle: React.CSSProperties = {
    border: `1px solid ${colors.border}`,
    backgroundColor: `${colors.surface}80`,
    color: colors.text,
  };
  const handleFocus = (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    e.currentTarget.style.borderColor = `${colors.cyan}80`;
  };
  const handleBlur = (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    e.currentTarget.style.borderColor = colors.border;
  };

  return (
    <div className="space-y-3">
      <style>{`.skill-editor-input::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
      <div>
        <label
          className="block text-[10px] uppercase mb-1"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Name
        </label>
        <input
          type="text"
          value={name}
          onChange={e => setName(e.target.value)}
          className="skill-editor-input w-full rounded-md px-3 py-1.5 text-xs focus:outline-none transition"
          style={inputStyle}
          onFocus={handleFocus}
          onBlur={handleBlur}
          placeholder="Skill name"
        />
      </div>
      <div>
        <label
          className="block text-[10px] uppercase mb-1"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Description
        </label>
        <textarea
          value={description}
          onChange={e => setDescription(e.target.value)}
          rows={3}
          className="skill-editor-input w-full rounded-md px-3 py-1.5 text-xs focus:outline-none transition resize-none"
          style={inputStyle}
          onFocus={handleFocus}
          onBlur={handleBlur}
          placeholder="What does this skill do?"
        />
      </div>
      <div className="flex items-center gap-2">
        <Button
          colors={colors}
          variant="ghostOn"
          type="button"
          onClick={handleSave}
          disabled={!hasChanges || !name.trim() || saving}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-border': 'transparent',
            '--pa-btn-bg-hover': `${colors.cyan}4D`,
            '--pa-btn-border-hover': 'transparent',
            '--pa-btn-bg-active': colors.cyanSoft,
            '--pa-btn-pad': '4px 12px',
            '--pa-btn-radius': `${radius.sm}px`,
            fontFamily: font.mono,
          } as CSSProperties}
        >
          {/* `Button` folds its children into one label span, so the icon and
              the word keep their own row to hold the `gap-1` they had. */}
          <span className="inline-flex items-center gap-1">
            <FiCheck size={12} />
            {saving ? 'Saving...' : 'Save'}
          </span>
        </Button>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          onClick={onClose}
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
            '--pa-btn-pad': '4px 12px',
            '--pa-btn-radius': `${radius.sm}px`,
            fontFamily: font.mono,
          } as CSSProperties}
        >
          <span className="inline-flex items-center gap-1">
            <FiX size={12} />
            Cancel
          </span>
        </Button>
      </div>
      {error && (
        <p className="text-[10px] text-red-400" style={{ fontFamily: font.mono }}>{error}</p>
      )}
    </div>
  );
}
