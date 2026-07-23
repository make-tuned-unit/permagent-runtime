import React, { lazy, Suspense } from 'react'
import ReactDOM from 'react-dom/client'
import './index.css'

const viewParam = new URLSearchParams(window.location.search).get('view');
const Root = lazy(() => {
  if (viewParam === 'chat') return import('./ChatApp');
  if (viewParam === 'pane') return import('./PaneWindowApp');
  if (viewParam === 'cyborg-test') return import('./CyborgTest');
  if (viewParam === 'stargate-test') return import('./StargateTest');
  return import('./App');
});

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Suspense fallback={null}>
      <Root />
    </Suspense>
  </React.StrictMode>,
)
