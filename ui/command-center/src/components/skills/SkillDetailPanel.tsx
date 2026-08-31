import { useState, type CSSProperties } from 'react';
import { FiTrash2, FiEdit2, FiX, FiChevronDown, FiChevronRight } from 'react-icons/fi';
import { useCommandCenter, type SkillState } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { SkillEditor } from './SkillEditor';
import { SkillExecutionHistory } from './SkillExecutionHistory';

interface SkillDetailPanelProps {
  skill: SkillState;
}

export function SkillDetailPanel({ skill }: SkillDetailPanelProps) {
  const { colors } = useTheme();
  const deleteSkill = useCommandCenter(s => s.deleteSkill);
  const setSelectedSkillId = useCommandCenter(s => s.setSelectedSkillId);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [showDefinition, setShowDefinition] = useState(false);
  const [deleteFailed, setDeleteFailed] = useState(false);

  // The skill staying put used to be the only sign a delete had failed, and
  // it looks exactly like a delete that was never attempted.
  const handleDelete = async () => {
    setDeleteFailed(false);
    const ok = await deleteSkill(skill.id);
    if (!ok) {
      setDeleteFailed(true);
      return false;
    }
    setSelectedSkillId(null);
    return true;
  };

  const dangerVars = {
    '--pa-btn-bg': `${colors.danger}33`,
    '--pa-btn-fg': colors.danger,
    '--pa-btn-border': 'transparent',
    '--pa-btn-bg-hover': `${colors.danger}4D`,
    '--pa-btn-border-hover': 'transparent',
    '--pa-btn-bg-active': `${colors.danger}33`,
    fontFamily: font.mono,
  } as CSSProperties;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        <div className="flex items-center gap-2 min-w-0">
          <span className={`w-2 h-2 rounded-full shrink-0 ${
            skill.status === 'active' ? 'bg-emerald-400' : 'bg-slate-500'
          }`} />
          <span
            className="text-sm truncate"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}
          >
            {skill.name}
          </span>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => setEditing(!editing)}
            className="rounded p-1.5 transition"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; e.currentTarget.style.backgroundColor = ''; }}
            title="Edit skill"
          >
            <FiEdit2 size={13} />
          </button>
          <button
            onClick={() => setConfirmDelete(true)}
            className="rounded p-1.5 transition"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.danger; e.currentTarget.style.backgroundColor = `${colors.danger}1A`; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; e.currentTarget.style.backgroundColor = ''; }}
            title="Delete skill"
          >
            <FiTrash2 size={13} />
          </button>
          <button
            onClick={() => setSelectedSkillId(null)}
            className="rounded p-1.5 hover:bg-white/5 transition"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
            title="Close"
          >
            <FiX size={14} />
          </button>
        </div>
      </div>

      {/* Delete confirmation */}
      {confirmDelete && (
        <div className="mx-4 mt-3 rounded-lg border border-red-400/30 bg-red-400/5 p-3">
          <p className="text-xs text-red-300 mb-2" style={{ fontFamily: font.mono }}>Delete this skill permanently?</p>
          <div className="flex gap-2">
            <Button colors={colors} type="button" onClick={handleDelete} style={dangerVars}>
              Delete
            </Button>
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => { setDeleteFailed(false); setConfirmDelete(false); }}
              style={{ fontFamily: font.mono, color: colors.textMuted }}
            >
              Cancel
            </Button>
          </div>
          {deleteFailed && (
            <p role="alert" className="text-[11px] mt-2" style={{ fontFamily: font.body, color: colors.danger }}>
              Couldn't delete this skill — it's still here. Try again.
            </p>
          )}
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {editing ? (
          <SkillEditor skill={skill} onClose={() => setEditing(false)} />
        ) : (
          <>
            {/* Description */}
            {skill.description && (
              <div>
                <label
                  className="block text-[10px] uppercase mb-1"
                  style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
                >
                  Description
                </label>
                <p className="text-xs" style={{ fontFamily: font.body, color: colors.text, opacity: 0.8 }}>{skill.description}</p>
              </div>
            )}

            {/* Metadata grid */}
            <div className="grid grid-cols-2 gap-3">
              <MetaField label="Trigger Type" value={skill.trigger_type || 'manual'} />
              <MetaField label="Status" value={skill.status || 'active'} />
              <MetaField label="Usage Count" value={String(skill.usageCount ?? 0)} />
              <MetaField label="Version" value={skill.version || '1'} />
              <MetaField label="Created" value={skill.created_at ? new Date(skill.created_at).toLocaleString() : '--'} />
              <MetaField label="Last Run" value={skill.last_run ? new Date(skill.last_run).toLocaleString() : 'Never'} />
            </div>

            {/* Steps */}
            {skill.steps && skill.steps.length > 0 && (
              <div>
                <label
                  className="block text-[10px] uppercase mb-2"
                  style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
                >
                  Steps
                </label>
                <div className="space-y-1.5">
                  {skill.steps.map((step, i) => (
                    <div key={i} className="flex items-start gap-2 rounded p-2" style={{ backgroundColor: `${colors.surface}80` }}>
                      <span className="text-[10px] shrink-0 mt-0.5" style={{ fontFamily: font.mono, color: colors.textMuted }}>{i + 1}.</span>
                      <div className="min-w-0">
                        <span className="text-xs" style={{ fontFamily: font.body, color: colors.text }}>{step.action}</span>
                        {step.tool && (
                          <span className="ml-2 text-[10px]" style={{ fontFamily: font.mono, color: colors.cyan, opacity: 0.7 }}>{step.tool}</span>
                        )}
                        {step.description && (
                          <p className="text-[10px] mt-0.5" style={{ fontFamily: font.body, color: colors.textMuted }}>{step.description}</p>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Definition JSON (collapsible) */}
            <div>
              <button
                onClick={() => setShowDefinition(!showDefinition)}
                className="flex items-center gap-1 text-[10px] uppercase transition"
                style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
                onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
                onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
              >
                {showDefinition ? <FiChevronDown size={11} /> : <FiChevronRight size={11} />}
                Definition JSON
              </button>
              {showDefinition && (
                <pre
                  className="mt-2 rounded p-3 text-[10px] overflow-x-auto max-h-64 overflow-y-auto"
                  style={{ fontFamily: font.mono, backgroundColor: `${colors.surface}80`, color: colors.textMuted }}
                >
                  {JSON.stringify(skill, null, 2)}
                </pre>
              )}
            </div>

            {/* Execution history */}
            <SkillExecutionHistory skillId={skill.id} />
          </>
        )}
      </div>
    </div>
  );
}

function MetaField({ label, value }: { label: string; value: string }) {
  const { colors } = useTheme();
  return (
    <div>
      <label
        className="block text-[10px] uppercase mb-0.5"
        style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
      >
        {label}
      </label>
      <span className="text-xs" style={{ fontFamily: font.mono, color: colors.text, opacity: 0.8 }}>{value}</span>
    </div>
  );
}
