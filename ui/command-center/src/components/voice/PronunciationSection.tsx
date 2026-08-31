import { useCallback, useEffect, useState } from 'react';
import { api, type PronunciationEntry, type UnresolvedPronunciation } from '../../lib/api';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Row, TextInput } from '../settings/atoms';

type C = ReturnType<typeof useTheme>['colors'];

const btn = (colors: C): React.CSSProperties => ({
  height: 30, padding: '0 12px', borderRadius: radius.md,
  background: colors.inputBg, border: `1px solid ${colors.border}`,
  color: colors.text, fontFamily: font.body, fontSize: 12,
  cursor: 'pointer', whiteSpace: 'nowrap',
});

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

  const teach = async (w: string, like: string) => {
    const trimmedWord = w.trim();
    const trimmedLike = like.trim();
    if (!trimmedWord || !trimmedLike) return;
    setBusy(true);
    setError(null);
    try {
      await api.savePronunciation(trimmedWord, trimmedLike);
      setWord('');
      setSoundsLike('');
      await reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not save pronunciation');
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
            <button
              type="button"
              disabled={busy || !word.trim() || !soundsLike.trim()}
              onClick={() => void teach(word, soundsLike)}
              style={btn(colors)}
            >
              Save
            </button>
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
                onTeach={(like) => void teach(item.word, like)}
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
                <button
                  type="button"
                  onClick={() => { void api.deletePronunciation(w).then(reload); }}
                  style={{ ...btn(colors), height: 26, padding: '0 8px', marginLeft: 'auto' }}
                >
                  Remove
                </button>
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
  onTeach: (soundsLike: string) => void;
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
      <button
        type="button"
        disabled={busy || !like.trim()}
        onClick={() => onTeach(like)}
        style={btn(colors)}
      >
        Teach
      </button>
    </div>
  );
}
