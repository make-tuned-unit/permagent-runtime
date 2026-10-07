import React from 'react';
import ReactDOM from 'react-dom/client';
import './index.css';
localStorage.setItem('permagent-daemon-token', 'x');
const q = new URLSearchParams(location.search);
const manifests = [
  { type: 'weather', name: 'Weather', description: '', defaultSize: { w: 3, h: 2 }, layout: 'compact', dataEndpoint: '/api/dashboard/weather', refreshSeconds: 900, source: 'built-in' },
  { type: 'system_stats', name: 'System', description: '', defaultSize: { w: 3, h: 2 }, layout: 'compact', dataEndpoint: '/api/dashboard/system-stats', refreshSeconds: 30, source: 'built-in' },
  { type: 'henry_status', name: 'Status', description: '', defaultSize: { w: 12, h: 2 }, layout: 'compact', dataEndpoint: '/api/dashboard/henry-status', source: 'built-in' },
];
const item = (t: string, r: string) => ({ kind: 'decision', refId: t, projectId: null, title: t, reason: r });
const routes: Record<string, unknown> = {
  '/api/dashboard/card-types': manifests,
  '/api/dashboard/layout': { cards: [
    { id: 'henry_status', type: 'henry_status', position: { x: 0, y: 0 }, size: { w: 12, h: 2 }, visible: true },
    { id: 'decisions', type: 'decisions', position: { x: 0, y: 2 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'today_focus', type: 'today_focus', position: { x: 5, y: 2 }, size: { w: 7, h: 4 }, visible: true },
    { id: 'in_flight', type: 'in_flight', position: { x: 0, y: 6 }, size: { w: 12, h: 3 }, visible: true },
    { id: 'weather', type: 'weather', position: { x: 0, y: 9 }, size: { w: 3, h: 2 }, visible: true },
    { id: 'system_stats', type: 'system_stats', position: { x: 3, y: 9 }, size: { w: 3, h: 2 }, visible: true },
  ] },
  '/api/dashboard': { agent: { name: 'Agent', state: 'idle', active_count: 0, summary: '' }, stats: { sessions_today: 3, sessions_total: 90, memory_count: 400, memory_delta_today: 4 }, in_flight: [], recent: [] },
  '/api/dashboard/insights': {
    todayFocus: { top: item('Approve the pricing change for the Atlas launch before the weekly review', 'the oldest of 3 decisions waiting, from 2d ago'), runnersUp: [item('Reply to the vendor about the renewal terms', 'stalled the longest of 2 held goals, shares its blocker with 1 other goal'), item('Pick a date for the offsite', 'waiting 1d')] },
    overnightDigest: { finished: [], stalledNew: [], newDecisions: [], newMemories: 0, quiet: true },
    fleetKpis: { projectCount: 3, activeGoalCount: 2, stalledGoalCount: 1, pendingDecisionCount: 3, sessionsToday: 3 },
    stalledPatterns: [], projectHealthGrid: [],
  },
  '/api/dashboard/weather': { cells: [
    { label: 'Now', value: '14°C', icon: 'sun', accent: true }, { label: 'Humidity', value: '77%', icon: 'drop' }, { label: 'Wind', value: '12 km/h' },
    { label: 'Thu', value: '15°', group: 'forecast' }, { label: 'Fri', value: '13°', group: 'forecast' }, { label: 'Sat', value: '12°', group: 'forecast' }, { label: 'Sun', value: '16°', group: 'forecast' } ] },
  '/api/dashboard/system-stats': { cells: [{ label: 'CPU load', value: '34%', icon: 'cpu', accent: true }, { label: 'Load average', value: '3.4 on 10 cores', icon: 'cpu' }, { label: 'Memory', value: '11.2 GB' }, { label: 'Disk free', value: '210 GB' }] },
  '/api/dashboard/henry-status': { cells: [{ label: 'Status', value: 'Idle', accent: true }, { label: 'Today', value: '3 sessions' }] },
};
routes['/api/people/directory'] = Array.from({ length: 31 }, (_, i) => ({
  entity_uuid: `p${i}`, display_name: `Person ${i}`, company: 'Acme', role: 'Lead', email: null, phone: null, notes: null,
  projects: i % 3 === 0 ? [] : [{ project_id: `pr${i % 4}`, project_name: `Project ${i % 4}` }], kind: 'person',
}));
const realFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const path = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href).pathname;
  if (path.startsWith('/api/') || path.startsWith('/config') || path.startsWith('/events')) {
    const body = path in routes ? routes[path] : (path.includes('decisions') ? [] : {});
    return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
  }
  return realFetch(input, init);
};
(window as any).__QA = q;
const view = q.get('view');
const Dashboard = view === 'people' ? (await import('./components/people/PeopleGraphCanvas')).PeopleGraph : (await import('./components/dashboard/Dashboard')).Dashboard;
ReactDOM.createRoot(document.getElementById('root')!).render(
  <div style={{ padding: 24, width: Number(q.get('w') || 1000), height: view ? 700 : undefined, boxSizing: 'border-box' }}><Dashboard /></div>
);
