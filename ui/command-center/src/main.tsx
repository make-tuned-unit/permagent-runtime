import React, { lazy, Suspense } from 'react'
import ReactDOM from 'react-dom/client'
import './index.css'

const viewParam = new URLSearchParams(window.location.search).get('view');
const Root = lazy(() => {
  if (viewParam === 'chat') return import('./ChatApp');
  if (viewParam === 'pane') return import('./PaneWindowApp');
  // Developer-only harness views: never reachable in a shipped build.
  if (import.meta.env.DEV && viewParam === 'cyborg-test') return import('./CyborgTest');
  if (import.meta.env.DEV && viewParam === 'stargate-test') return import('./StargateTest');
  return import('./App');
});

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Suspense fallback={null}>
      <Root />
    </Suspense>
  </React.StrictMode>,
)
