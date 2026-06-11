// Dev entry for the W2 prop catalog (served at /propcatalog.html in dev only).
import { createRoot } from 'react-dom/client';
import { PropCatalog } from './PropCatalog';

createRoot(document.getElementById('root')!).render(<PropCatalog />);
