import { createRoot } from 'react-dom/client';
import '../../src/index.css';
import { CalendarCard } from '../../src/components/dashboard/cards/CalendarCard';
import { setTheme } from '../../src/styles/tokens';

(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
const p = new URLSearchParams(location.search);
setTheme(p.get('theme') === 'light' ? 'silver' : 'dark');
const view = (p.get('view') ?? 'week') as 'day' | 'week' | 'month';
const cols = Number(p.get('cols') ?? 7);
const vw = window.innerWidth;
// Home's grid: the sidebar and page padding come off the viewport, 12 columns, 16px gaps.
const inner = vw - 260 - 48;
const col = (inner - 11 * 16) / 12;
const width = cols * col + (cols - 1) * 16;
const NOW = new Date(2026, 9, 7, 10, 18, 0).getTime();

createRoot(document.getElementById('root')!).render(
  <div style={{ padding: 24, marginLeft: 260, width, minHeight: '100vh' }} data-testid="harness-frame">
    <CalendarCard nowMs={NOW} initialView={view} />
  </div>,
);
