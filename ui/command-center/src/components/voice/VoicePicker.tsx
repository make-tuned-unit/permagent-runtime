import { useTheme } from '../../styles/useTheme';
import { font, radius } from '../../styles/tokens';
import { useVoices, useVoicePreview } from '../../lib/useVoices';

type C = ReturnType<typeof useTheme>['colors'];

const selectStyle = (colors: C): React.CSSProperties => ({
  height: 34, padding: '0 12px', borderRadius: radius.md,
  background: colors.inputBg, border: `1px solid ${colors.border}`,
  color: colors.text, fontFamily: font.body, fontSize: 13,
  flex: 1, cursor: 'pointer',
});
const btnStyle = (colors: C): React.CSSProperties => ({
  height: 34, padding: '0 14px', borderRadius: radius.md,
  background: colors.inputBg, border: `1px solid ${colors.border}`,
  color: colors.text, fontFamily: font.body, fontSize: 13,
  cursor: 'pointer', whiteSpace: 'nowrap',
});

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

  if (loading) return <span style={{ color: colors.textDim, fontSize: 13 }}>Loading voices…</span>;

  if (!ready) {
    const downloading = !!status?.downloading || (downloadPercent > 0 && downloadPercent < 100);
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <span style={{ color: colors.textDim, fontSize: 13 }}>
          Voice models aren’t downloaded yet (~353&nbsp;MB, one time). Download to enable spoken voice.
        </span>
        {downloading
          ? <div style={{ fontSize: 12, color: colors.textDim }}>Downloading… {downloadPercent}%</div>
          : <button onClick={startDownload} style={btnStyle(colors)}>Download voice models</button>}
        {downloadError && <span style={{ fontSize: 12, color: colors.danger }}>{downloadError}</span>}
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
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%' }}>
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
      <button
        onClick={() => preview(effective)}
        disabled={!!playingId}
        title="Preview this voice"
        style={btnStyle(colors)}
      >{playingId ? '…' : '▶ Preview'}</button>
      {previewError && <span style={{ fontSize: 12, color: colors.danger }}>{previewError}</span>}
    </div>
  );
}
