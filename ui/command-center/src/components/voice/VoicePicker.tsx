import { type CSSProperties } from 'react';
import { useTheme } from '../../styles/useTheme';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useVoices, useVoicePreview } from '../../lib/useVoices';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';

type C = ReturnType<typeof useTheme>['colors'];

const selectStyle = (colors: C): React.CSSProperties => ({
  height: 34, padding: '0 12px', borderRadius: radius.md,
  background: colors.inputBg, border: `1px solid ${colors.border}`,
  color: colors.text, fontFamily: font.body, fontSize: textSize.small,
  flex: 1, cursor: 'pointer',
});
/** The picker's two controls, expressed for the shared button primitive: the
 *  same resting look as the inline style it replaces, with hover/press/disabled
 *  coming from `.pa-btn`. */
const btnVars = (colors: C): CSSProperties => ({
  '--pa-btn-bg': colors.inputBg,
  '--pa-btn-fg': colors.text,
  '--pa-btn-border': colors.border,
  '--pa-btn-bg-hover': colors.surfaceHi,
  '--pa-btn-border-hover': colors.borderHi,
  '--pa-btn-bg-active': colors.surface,
  '--pa-btn-pad': '0 14px',
  '--pa-btn-radius': `${radius.md}px`,
  height: 34,
  fontFamily: font.body,
  fontSize: textSize.small,
  whiteSpace: 'nowrap',
} as CSSProperties);

/**
 * Data-driven voice picker over the loaded Kokoro pack with a per-voice audio
 * preview. Degrades gracefully to a download affordance when the ~353MB assets
 * aren't present yet (never a picker that can't synthesize — Step 0c). Shared
 * by the Settings persona panel and the first-run wizard.
 */
export function VoicePicker({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (v: string | null) => void;
}) {
  const { colors } = useTheme();
  const { voices, ready, loading, status, downloadPercent, downloadError, startDownload } = useVoices();
  const { preview, playingId, error: previewError } = useVoicePreview();

  if (loading) return <span style={{ color: colors.textDim, fontSize: textSize.small }}>Loading voices…</span>;

  if (!ready) {
    const downloading = !!status?.downloading || (downloadPercent > 0 && downloadPercent < 100);
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        <span style={{ color: colors.textDim, fontSize: textSize.small }}>
          Voice models aren’t downloaded yet (~353&nbsp;MB, one time). Download to enable spoken voice.
        </span>
        {downloading
          ? <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Downloading… {downloadPercent}%</div>
          : (
            // `startDownload` reports its own failure into `downloadError` and
            // resolves either way, so the tick could not tell the two apart.
            <Button
              colors={colors}
              onClick={startDownload}
              flashSuccess={false}
              style={btnVars(colors)}
            >
              Download voice models
            </Button>
          )}
        {downloadError && <span style={{ fontSize: textSize.caption, color: colors.danger }}>{downloadError}</span>}
      </div>
    );
  }

  // Seed bf_emma when nothing is selected yet (falls back to first available).
  const effective = value ?? (voices.some(v => v.id === 'bf_emma') ? 'bf_emma' : (voices[0]?.id ?? null));

  const groups = new Map<string, typeof voices>();
  for (const v of voices) {
    const key = v.language || 'Other';
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(v);
  }

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: space.md, width: '100%' }}>
      <select value={effective ?? ''} onChange={e => onChange(e.target.value)} style={selectStyle(colors)}>
        {[...groups.entries()].map(([lang, vs]) => (
          <optgroup key={lang} label={lang}>
            {vs.map(v => {
              const short = v.label.startsWith(lang) ? v.label.slice(lang.length).replace(/^[\s—-]+/, '') : v.label;
              return <option key={v.id} value={v.id}>{short || v.label}</option>;
            })}
          </optgroup>
        ))}
      </select>
      {/* The preview resolves when playback STARTS, and reports failure into
          `previewError` rather than by rejecting — a tick would be a guess. */}
      <Tooltip content="Preview this voice">
        <Button
          colors={colors}
          onClick={() => preview(effective)}
          disabled={!!playingId}
          flashSuccess={false}
          style={btnVars(colors)}
        >{playingId ? '…' : '▶ Preview'}</Button>
      </Tooltip>
      {previewError && <span style={{ fontSize: textSize.caption, color: colors.danger }}>{previewError}</span>}
    </div>
  );
}
