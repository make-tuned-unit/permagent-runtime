import React, { lazy, Suspense } from 'react'
import ReactDOM from 'react-dom/client'
import './index.css'

const isChatWindow = new URLSearchParams(window.location.search).get('view') === 'chat';
const Root = lazy(() => isChatWindow ? import('./ChatApp') : import('./App'));

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Suspense fallback={null}>
      <Root />
    </Suspense>
  </React.StrictMode>,
)
