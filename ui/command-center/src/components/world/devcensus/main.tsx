// Dev entry for the coordinator World V2 census + evidence harness.
// Served at /worldcensus.html in dev only (vite build emits index.html only).
// Mounts the real integrated <WorldView/> so the CDP harness can read
// window.__worldScene (light census) + window.__worldPerf (fps/calls/tris) and
// capture screenshots of the fully-integrated scene.
import { createRoot } from 'react-dom/client';
import { WorldView } from '../WorldView';

createRoot(document.getElementById('root')!).render(<WorldView visible />);
