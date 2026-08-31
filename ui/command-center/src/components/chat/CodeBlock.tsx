import { useState, useEffect, useCallback, useRef, type CSSProperties } from 'react';
import { FiCopy, FiCheck, FiAlertCircle } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCopyToClipboard } from '../../lib/clipboard';
import { Button } from '../common/Button';

/** How long a block must stop growing before it is worth re-colouring. Well
 *  under the gap between a stream ending and the reader's eye reaching the
 *  block, and irrelevant to already-settled blocks (see below). */
const HIGHLIGHT_SETTLE_MS = 150;

export function CodeBlock({ language, code }: { language: string; code: string }) {
  const { colors, theme } = useTheme();
  const shikiTheme = theme === 'silver' ? 'github-light' : 'github-dark';
  // The highlight is kept WITH the input it was produced from. Storing bare
  // html would let a colourised snapshot of an earlier, shorter version of a
  // streaming block stay on screen while the code kept growing — the block
  // would appear to freeze mid-stream, which is worse than no colour at all.
  const [highlight, setHighlight] = useState<{ code: string; theme: string; html: string } | null>(null);
  const { state: copyState, copy } = useCopyToClipboard();
  // Whether this block has been highlighted at least once — see the effect.
  const highlightedOnce = useRef(false);

  const html = highlight && highlight.code === code && highlight.theme === shikiTheme ? highlight.html : null;

  useEffect(() => {
    let cancelled = false;
    const run = () => {
      import('shiki').then(({ codeToHtml }) => {
        codeToHtml(code, { lang: language, theme: shikiTheme })
          .then(result => {
            if (!cancelled) setHighlight({ code, theme: shikiTheme, html: result });
          })
          .catch(() => {});
      });
    };

    // First pass runs immediately: a settled block — history, or a reply that
    // arrived whole — must never sit colourless waiting on a timer.
    if (!highlightedOnce.current) {
      highlightedOnce.current = true;
      run();
      return () => { cancelled = true; };
    }

    // A RE-highlight only ever happens because `code` grew, i.e. this block is
    // streaming in. shiki re-tokenises the WHOLE block every call, so doing it
    // per delta costs O(deltas x length): measured at 340ms of main-thread time
    // to bring a single 60-line TypeScript block on screen, with the final pass
    // alone taking 10.6ms — most of a 16ms frame, stolen from the paint that
    // shows the text. Waiting for the block to stop growing costs one pass
    // instead of sixty. Nothing is delayed to buy that: the <pre> below renders
    // the newest code on every delta regardless, so only the colours wait.
    const timer = setTimeout(run, HIGHLIGHT_SETTLE_MS);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [code, language, shikiTheme]);

  const handleCopy = useCallback(() => { void copy(code); }, [copy, code]);

  return (
    <div
      className="relative group rounded-lg my-2 overflow-hidden"
      style={{ backgroundColor: colors.codeBg, border: `1px solid ${colors.border}` }}
    >
      <div
        className="flex items-center justify-between px-3 py-1"
        style={{ backgroundColor: colors.surface, borderBottom: `1px solid ${colors.border}` }}
      >
        <span className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>{language}</span>
        <Button
          colors={colors}
          variant="bare"
          onClick={handleCopy}
          aria-label={`Copy ${language} code`}
          // focus-visible keeps the button reachable by keyboard: opacity-0
          // alone made it tabbable-but-invisible, which is worse than absent.
          className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            fontFamily: font.mono,
            fontSize: 10,
            gap: 4,
          } as CSSProperties}
        >
          {copyState === 'copied' ? <><FiCheck size={11} style={{ color: colors.success }} /> Copied</>
            : copyState === 'failed' ? <><FiAlertCircle size={11} style={{ color: colors.danger }} /> Copy failed</>
            : <><FiCopy size={11} /> Copy</>}
        </Button>
      </div>
      {html ? (
        <div
          className="overflow-x-auto p-3 text-[12px] leading-relaxed [&_pre]:!bg-transparent [&_pre]:!m-0 [&_pre]:!p-0 [&_code]:!text-[12px]"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="overflow-x-auto p-3 text-[12px] leading-relaxed" style={{ fontFamily: font.mono, color: colors.text }}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
