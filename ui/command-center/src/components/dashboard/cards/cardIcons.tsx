/**
 * Glyphs for manifest card cells.
 *
 * The daemon names an icon (`CardCell.icon`); this maps the name to a drawing.
 * The data source decides what a cell MEANS — the UI must not sniff it out of
 * display text, which breaks the moment a label is reworded or translated.
 *
 * All are 24x24 stroke paths so they inherit `currentColor` and match the
 * sidebar's line weight. SVG, never emoji: emoji render as someone else's
 * artwork at someone else's weight, and they land differently per platform.
 */

export type CardIconName =
  | 'clear' | 'partly-cloudy' | 'overcast' | 'fog' | 'drizzle' | 'rain'
  | 'snow' | 'thunderstorm'
  | 'cpu' | 'memory' | 'disk' | 'clock' | 'thermometer' | 'droplet';

/** Multi-path glyphs; each entry is a list of `d` strings. */
const PATHS: Record<CardIconName, string[]> = {
  clear: ['M12 4V2M12 22v-2M4 12H2M22 12h-2M5.6 5.6L4.2 4.2M19.8 19.8l-1.4-1.4M5.6 18.4l-1.4 1.4M19.8 4.2l-1.4 1.4', 'M12 7.5a4.5 4.5 0 100 9 4.5 4.5 0 000-9z'],
  'partly-cloudy': ['M8 6.5A3.5 3.5 0 0111.9 3a3.5 3.5 0 013.4 2.6', 'M6.5 20h10a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 006.5 20z'],
  overcast: ['M7 18h9.5a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 007 18z', 'M9 8a4 4 0 016.5-2'],
  fog: ['M6.5 15h10a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 006.5 15z', 'M4 18.5h16M6 21.5h12'],
  drizzle: ['M6.5 15h10a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 006.5 15z', 'M9 18v1.5M13 18v2.5M17 18v1.5'],
  rain: ['M6.5 14h10a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 006.5 14z', 'M8.5 17l-1 3.5M12.5 17l-1 3.5M16.5 17l-1 3.5'],
  snow: ['M6.5 14h10a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 006.5 14z', 'M9 18h.01M12 20h.01M15 18h.01M12 17h.01'],
  thunderstorm: ['M6.5 13.5h10a3.5 3.5 0 000-7 5 5 0 00-9.6 1.4A3 3 0 006.5 13.5z', 'M13 16l-3 4h3l-1 3.5'],
  cpu: ['M6.5 6.5h11v11h-11z', 'M9.5 9.5h5v5h-5z', 'M9 6.5V4M15 6.5V4M9 20v-2.5M15 20v-2.5M6.5 9H4M6.5 15H4M20 9h-2.5M20 15h-2.5'],
  memory: ['M4 8h16v8H4z', 'M8 8v8M12 8v8M16 8v8', 'M7 5v3M17 5v3'],
  disk: ['M12 3a9 9 0 100 18 9 9 0 000-18z', 'M12 9.5a2.5 2.5 0 100 5 2.5 2.5 0 000-5z'],
  clock: ['M12 3a9 9 0 100 18 9 9 0 000-18z', 'M12 7.5V12l3 2'],
  thermometer: ['M12 3a2 2 0 012 2v8.5a4 4 0 11-4 0V5a2 2 0 012-2z'],
  droplet: ['M12 3.5s5.5 6 5.5 9.5a5.5 5.5 0 11-11 0C6.5 9.5 12 3.5 12 3.5z'],
};

/** Map a daemon icon name to paths; unknown names render nothing rather than
 *  a wrong picture. A missing glyph is a smaller lie than a misleading one. */
export function cardIconPaths(name: string | undefined): string[] | null {
  if (!name) return null;
  return PATHS[name as CardIconName] ?? null;
}

export function CardIcon({ name, size = 16, color }: {
  name: string | undefined;
  size?: number;
  color?: string;
}) {
  const paths = cardIconPaths(name);
  if (!paths) return null;
  return (
    <svg
      width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden
      stroke={color ?? 'currentColor'} strokeWidth={1.6}
      strokeLinecap="round" strokeLinejoin="round"
      style={{ flexShrink: 0, display: 'block' }}
    >
      {paths.map((d, i) => <path key={i} d={d} />)}
    </svg>
  );
}
