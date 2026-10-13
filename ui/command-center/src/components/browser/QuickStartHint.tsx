import { type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { Button } from '../common/Button';
import { CHROME_GEOM, chromeBareVars } from './browserChrome';

/** Host of a search template, for display only. */
export function templateHost(template: string): string {
  try { return new URL(template.replace('%s', 'x')).host; } catch { return 'your search template'; }
}

/**
 * What an empty browser offers. No vendor is hardcoded: with a user-set search
 * template it offers that template; without one it says how to set one.
 */
export function QuickStartHint({ template, colors, onFocusAddress }: {
  template: string | null;
  colors: ThemeColors;
  onFocusAddress: () => void;
}) {
  if (!template) {
    return (
      <p data-testid="browser-quickstart-hint" className="mt-6 text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>
        Enter a URL, or set a search template in Settings to search from the address bar.
      </p>
    );
  }
  return (
    <div className="mt-6 flex flex-wrap gap-2 justify-center">
      <Button
        colors={colors}
        variant="bare"
        data-testid="browser-quickstart-search"
        onClick={onFocusAddress}
        style={{
          ...chromeBareVars(colors, { bg: colors.fillSubtle, pad: `${CHROME_GEOM.tabPadY}px ${CHROME_GEOM.tabPadX}px`, radiusPx: radius.sm }),
          '--pa-btn-bg-hover': colors.cyanSoft,
          '--pa-btn-fg-hover': colors.cyan,
          '--pa-btn-bg-active': colors.cyanGlow,
          fontFamily: font.body,
          fontSize: textSize.caption,
        } as CSSProperties}
      >
        Search with {templateHost(template)}
      </Button>
    </div>
  );
}
