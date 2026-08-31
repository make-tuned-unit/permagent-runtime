import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { api, type PronunciationEntry, type UnresolvedPronunciation } from '../../lib/api';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Row, TextInput } from '../settings/atoms';
import { Button } from '../common/Button';

type C = ReturnType<typeof useTheme>['colors'];

/** The section's one button look, expressed for `.pa-btn`: identical at rest,
 *  with hover/press/pending/disabled arriving from the primitive. */
const btnVars = (colors: C): CSSProperties => ({
  '--pa-btn-bg': colors.inputBg,
  '--pa-btn-fg': colors.text,
  '--pa-btn-border': colors.border,
  '--pa-btn-bg-hover': colors.surfaceHi,
  '--pa-btn-border-hover': colors.borderHi,
  '--pa-btn-bg-active': colors.surface,
  '--pa-btn-pad': '0 12px',
  '--pa-btn-radius': `${radius.md}px`,
  height: 30,
  fontFamily: font.body,
  fontSize: 12,
  whiteSpace: 'nowrap',
} as CSSProperties);

/**
 * Teach the speech engine a word once, and review anything it had to guess at.
 * Conversational `save_pronunciation` is the primary path; this is the review
 * queue so a coined name can be fixed before the next demo.
 */
export function PronunciationSection() {
  const { colors } = useTheme();
  const [saved, setSaved] = useState<Record<string, PronunciationEntry>>({});
  const [unresolved, setUnresolved] = useState<UnresolvedPronunciation[]>([]);
  const [word, setWord] = useState('');
  const [soundsLike, setSoundsLike] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [lex, queue] = await Promise.all([
        api.getPronunciations(),
        api.getUnresolvedPronunciations(),
      ]);
      setSaved(lex);
      setUnresolved(queue.unresolved);
    } catch {
      // Voice routes unavailable — section stays empty.
    }
  }, []);

  useEffect(() => { void reload(); }, [reload]);

  /** Resolves `false` when nothing was saved — this swallows its own error into
   *  `error`, and a button that ticked on that would be claiming a save that
   *  did not happen. */
  const teach = async (w: string, like: string): Promise<boolean> => {
    const trimmedWord = w.trim();
    const trimmedLike = like.trim();
    if (!trimmedWord || !trimmedLike) return false;
    setBusy(true);
    setError(null);
    try {
      await api.savePronunciation(trimmedWord, trimmedLike);
      setWord('');
      setSoundsLike('');
      await reload();
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not save pronunciation');
      return false;
    } finally {
      setBusy(false);
    }
  };

  const savedWords = Object.keys(saved).sort();

  return (
    <>
      <Row
        label="Teach a word"
        hint="Respell with real English words, the way you'd tell a person. 'per ma gent', not IPA."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            <div style={{ flex: 1, minWidth: 120 }}>
              <TextInput value={word} onChange={setWord} placeholder="word" />
            </div>
            <div style={{ flex: 2, minWidth: 160 }}>
              <TextInput value={soundsLike} onChange={setSoundsLike} placeholder="sounds like" />
            </div>
            <Button
              colors={colors}
              type="button"
              disabled={busy || !word.trim() || !soundsLike.trim()}
              onClick={() => teach(word, soundsLike)}
              style={btnVars(colors)}
            >
              Save
            </Button>
          </div>
          {error && <span style={{ fontSize: 12, color: colors.danger }}>{error}</span>}
        </div>
      </Row>
      {unresolved.length > 0 && (
        <Row
          label="Needs a reading"
          hint="Spoken with a guess last time. Teach each once — it's remembered."
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {unresolved.map(item => (
              <UnresolvedRow
                key={item.word}
                item={item}
                colors={colors}
                busy={busy}
                onTeach={(like) => teach(item.word, like)}
              />
            ))}
          </div>
        </Row>
      )}
      {savedWords.length > 0 && (
        <Row label="Saved" hint="Applied on the next spoken sentence.">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {savedWords.map(w => (
              <div key={w} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, fontFamily: font.body }}>
                <span style={{ color: colors.text, fontWeight: 500 }}>{w}</span>
                <span style={{ color: colors.textMuted }}>{saved[w].sounds_like}</span>
                <Button
                  colors={colors}
                  type="button"
                  onClick={() => api.deletePronunciation(w).then(reload)}
                  style={{ ...btnVars(colors), height: 26, '--pa-btn-pad': '0 8px', marginLeft: 'auto' } as CSSProperties}
                >
                  Remove
                </Button>
              </div>
            ))}
          </div>
        </Row>
      )}
    </>
  );
}

function UnresolvedRow({
  item,
  colors,
  busy,
  onTeach,
}: {
  item: UnresolvedPronunciation;
  colors: C;
  busy: boolean;
  /** Returns the save's promise so the button can spin for it and tick only on
   *  a real save. */
  onTeach: (soundsLike: string) => unknown;
}) {
  const [like, setLike] = useState('');
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
      <span style={{ fontSize: 13, fontFamily: font.body, color: colors.text, minWidth: 80 }}>
        {item.word}
      </span>
      <span style={{ fontSize: 11, color: colors.textMuted }}>×{item.spelled_out_times}</span>
      <div style={{ flex: 1, minWidth: 140 }}>
        <TextInput value={like} onChange={setLike} placeholder="sounds like" />
      </div>
      <Button
        colors={colors}
        type="button"
        disabled={busy || !like.trim()}
        onClick={() => onTeach(like)}
        style={btnVars(colors)}
      >
        Teach
      </Button>
    </div>
  );
}
