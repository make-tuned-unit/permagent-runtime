import { useState } from 'react';
import { FiCheck, FiX } from 'react-icons/fi';
import { useCommandCenter, type SkillState } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

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

  const handleSave = async () => {
    if (!hasChanges || !name.trim()) return;
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
        <button
          onClick={handleSave}
          disabled={!hasChanges || !name.trim() || saving}
          className="flex items-center gap-1 rounded-md px-3 py-1 text-[11px] transition disabled:opacity-40 disabled:cursor-not-allowed"
          style={{ fontFamily: font.mono, backgroundColor: colors.cyanSoft, color: colors.cyan }}
          onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.cyan}4D`; }}
          onMouseLeave={e => { e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
        >
          <FiCheck size={12} />
          {saving ? 'Saving...' : 'Save'}
        </button>
        <button
          onClick={onClose}
          className="flex items-center gap-1 rounded-md px-3 py-1 text-[11px] hover:bg-white/5 transition"
          style={{ fontFamily: font.mono, color: colors.textMuted }}
          onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
          onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
        >
          <FiX size={12} />
          Cancel
        </button>
      </div>
      {error && (
        <p className="text-[10px] text-red-400" style={{ fontFamily: font.mono }}>{error}</p>
      )}
    </div>
  );
}
