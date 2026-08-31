import { useState, useCallback, type CSSProperties } from 'react';
import { JsonView, darkStyles, defaultStyles } from 'react-json-view-lite';
import 'react-json-view-lite/dist/index.css';
import { FiCopy, FiCheck } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

interface JsonResultProps {
  data: unknown;
}

export function JsonResult({ data }: JsonResultProps) {
  const { colors, theme } = useTheme();
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(JSON.stringify(data, null, 2)).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [data]);

  return (
    <div className="relative group">
      <Button
        colors={colors}
        variant="bare"
        onClick={handleCopy}
        // Position and reveal-on-hover are this call site's, not the
        // primitive's — `Button` merges className after `pa-btn`.
        className="absolute top-1 right-1 opacity-0 group-hover:opacity-100 z-10"
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
        {copied ? <><FiCheck size={10} style={{ color: colors.success }} /> Copied</> : <><FiCopy size={10} /> Copy JSON</>}
      </Button>
      <div className="overflow-x-auto max-h-[300px] overflow-y-auto text-[11px] [&_*]:!font-mono">
        <JsonView data={data as object} style={theme === 'silver' ? defaultStyles : darkStyles} />
      </div>
    </div>
  );
}
