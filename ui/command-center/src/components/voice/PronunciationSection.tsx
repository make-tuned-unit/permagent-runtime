import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { api, type PronunciationEntry, type UnknownWord } from '../../lib/api';
import { font, radius, space, textSize } from '../../styles/tokens';
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
  fontSize: textSize.caption,
  whiteSpace: 'nowrap',
} as CSSProperties);

/**
 * Set how a word sounds, and review words the voice had to guess at.
 * Spoken turns never ask about pronunciation; this list is where a coined name
 * gets fixed.
 */
export function PronunciationSection() {
  const { colors } = useTheme();
  const [saved, setSaved] = useState<Record<string, PronunciationEntry>>({});
  const [unknown, setUnknown] = useState<UnknownWord[]>([]);
  const [word, setWord] = useState('');
  const [soundsLike, setSoundsLike] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [lex, queue] = await Promise.all([
        api.getPronunciations(),
        api.getUnknownWords().catch(() => ({ words: [] as UnknownWord[], max_words: 0 })),
      ]);
      setSaved(lex);
      setUnknown(queue.words);
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

  const dismiss = async (w: string) => {
    setError(null);
    try {
      await api.dismissUnknownWord(w);
      await reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not dismiss that word');
    }
  };

  const savedWords = Object.keys(saved).sort();

  return (
    <>
      <Row
        label="Teach a word"
        hint="Respell with real English words, the way you'd tell a person. 'per ma gent', not IPA."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          <div style={{ display: 'flex', gap: space.md, flexWrap: 'wrap' }}>
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
          {error && <span style={{ fontSize: textSize.caption, color: colors.danger }}>{error}</span>}
        </div>
      </Row>
      <Row
        label="Words to check"
        hint="Henry came across these and said them his own way. Tell him how a word sounds, or dismiss it if it was fine."
      >
        {unknown.length === 0 ? (
          <span style={{ fontSize: textSize.small, fontFamily: font.body, color: colors.textMuted }}>
            Nothing to check. Words Henry is unsure how to say will show up here.
          </span>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
            {unknown.map(item => (
              <UnknownWordRow
                key={item.word}
                item={item}
                colors={colors}
                busy={busy}
                onTeach={(like) => teach(item.word, like)}
                onDismiss={dismiss}
              />
            ))}
          </div>
        )}
      </Row>
      {savedWords.length > 0 && (
        <Row label="Saved" hint="Applied on the next spoken sentence.">
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
            {savedWords.map(w => (
              // The respelling takes the slack, not the button: with neither
              // side allowed to grow, a narrow row squeezed "bar dee" onto two
              // lines while "Remove" kept its margin.
              <div key={w} style={{ display: 'flex', alignItems: 'center', gap: space.md, fontSize: textSize.small, fontFamily: font.body }}>
                <span style={{ color: colors.text, fontWeight: 500, flexShrink: 0 }}>{w}</span>
                <span style={{ color: colors.textMuted, flex: 1, minWidth: 0, overflowWrap: 'anywhere' }}>{saved[w].sounds_like}</span>
                <Button
                  colors={colors}
                  type="button"
                  onClick={() => api.deletePronunciation(w).then(reload)}
                  style={{ ...btnVars(colors), height: 26, '--pa-btn-pad': '0 8px', flexShrink: 0 } as CSSProperties}
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

function UnknownWordRow({
  item,
  colors,
  busy,
  onTeach,
  onDismiss,
}: {
  item: UnknownWord;
  colors: C;
  busy: boolean;
  /** Returns the save's promise so the button ticks only on a real save. */
  onTeach: (soundsLike: string) => unknown;
  onDismiss: (word: string) => unknown;
}) {
  const [like, setLike] = useState('');
  const [open, setOpen] = useState(false);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
      <div style={{ display: 'flex', gap: space.md, alignItems: 'center', flexWrap: 'wrap' }}>
        <span style={{ fontSize: textSize.small, fontFamily: font.body, color: colors.text, fontWeight: 500 }}>
          {item.display || item.word}
        </span>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>
          {item.count === 1 ? 'said once' : `said ${item.count} times`}
        </span>
        <span style={{ flex: 1 }} />
        <Button colors={colors} type="button" onClick={() => setOpen(o => !o)} style={btnVars(colors)}>
          Set pronunciation
        </Button>
        <Button colors={colors} type="button" onClick={() => onDismiss(item.word)} style={btnVars(colors)}>
          Dismiss
        </Button>
      </div>
      {item.sentence && (
        <span style={{ fontSize: textSize.caption, color: colors.textMuted, overflowWrap: 'anywhere' }}>
          {item.sentence}
        </span>
      )}
      {open && (
        <div style={{ display: 'flex', gap: space.md, alignItems: 'center', flexWrap: 'wrap' }}>
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
            Save
          </Button>
        </div>
      )}
    </div>
  );
}
