// Local visual QA only. No real credentials, daemon requests or user data.
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../src/index.css';
import { Dashboard } from '../src/components/dashboard/Dashboard';
import { DEFAULT_LAYOUT, LEGACY_MORNING_LAYOUT } from '../src/components/dashboard/useLayout';
import { useCommandCenter } from '../src/lib/store';
import { setTheme } from '../src/styles/tokens';

const params = new URLSearchParams(location.search);
const empty = params.get('state') === 'empty';
const busy = params.get('state') === 'busy';
const now = new Date().toISOString();
const today = new Date().toLocaleDateString('en-CA');
let layout = structuredClone(params.has('legacy') ? LEGACY_MORNING_LAYOUT : DEFAULT_LAYOUT);
localStorage.setItem('permagent-daemon-token', 'synthetic-fixture-not-a-credential');
// Every fetch is handled locally, including unrecognized routes and writes.
window.fetch = async (input, init) => {
  const path = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href).pathname;
  const method = init?.method ?? 'GET';
  let data: unknown;
  if (path === '/api/dashboard/layout') {
    if (method === 'PUT') layout = JSON.parse(String(init?.body));
    data = layout;
  } else if (path === '/api/dashboard') data = {
    agent: { name: 'Aria', state: 'idle', active_count: empty ? 0 : 1, summary: '' },
    stats: { sessions_today: 3, sessions_total: 12, memory_count: 20, memory_delta_today: 2 },
    in_flight: [], recent: empty ? [] : Array.from({length: busy ? 12 : 3}, (_, i) => ({ id: `session-${i}`, title: ['Plan the next release', 'Ideas for the garden', 'Review the project brief'][i % 3], name: ['Plan the next release', 'Ideas for the garden', 'Review the project brief'][i % 3], session_type: 'user', state: 'idle', ended_at: now })),
  };
  else if (path === '/api/agent/identity') data = { first_name: 'Aria', display_name: 'Aria', traits: [], tone: 'warm', opening_greeting: '' };
  else if (path === '/api/decisions') {
    if (params.get('state') === 'error') return new Response('{}', {status: 503});
    data = { items: empty ? [] : Array.from({length: 8}, (_, i) => ({ id: `fixture-decision-${i}`, kind: 'approve_review', headline: busy ? 'Review the accessibility improvements and the updated navigation across the complete project before the next release is published' : 'Review the updated project plan', detail: 'The draft is ready for your review.', created_at: now, status: 'open', payload: {}, tier: 2, goal_id: null, project_id: null })), summary: { total_pending: empty ? 0 : 8, handled_count: 3, goals_in_flight: empty ? 0 : 1, goals_needing_attention: 0, oldest_pending_at: now }, attentionGoals: [] };
  } else if (path === '/api/cards/due') data = empty ? [] : Array.from({length: busy ? 15 : 3}, (_, i) => ({id: `todo-${i}`, title: ['Review the launch checklist', 'Prepare the next design review', 'Send the updated brief'][i % 3], projectId: 'sample', projectName: 'Sample project', columnId: 'todo', columnName: 'To do', dueDate: today, assignedTo: null, updatedAt: now}));
  else if (path === '/api/goals/active') data = {count: empty ? 0 : 1, goals: empty ? [] : [{id: 'fixture-goal', title: 'Prepare the release notes', project_id: 'sample', state: 'in_progress', assigned_to: null, created_at: now, updated_at: now, auto_approve: false}]};
  else if (path === '/api/dashboard/calendar') data = {cells: []};
  else if (path === '/api/dashboard/card-types') data = [];
  else if (path === '/api/dashboard/templates') data = [];
  else if (path === '/api/onboarding/status') data = {learn_next: [], totals: {used: 8, teachable: 8}};
  else if (path === '/api/brain/graph') data = {nodes: [], edges: []};
  else if (path === '/api/decisions/history') data = {items: [], nextBefore: null};
  else if (path === '/sse-token') data = {token: 'fixture'};
  else return new Response('{}', {status: 404});
  return new Response(JSON.stringify(data), {headers: {'Content-Type': 'application/json'}});
};
// No event socket may reach a daemon from the harness.
window.WebSocket = class { close() {} send() {} } as unknown as typeof WebSocket;
useCommandCenter.setState({currentModel: 'fixture-model', agentName: 'Aria', chatResolvedRoute: {provider: 'fixture', model: 'fixture', source: 'default'} as never, loadProviders: async () => {}});
setTheme(params.get('theme') === 'silver' ? 'silver' : 'dark');
const width = Number(params.get('width')) || undefined;
createRoot(document.getElementById('root')!).render(<div style={{height: '100vh', width: width ? Math.min(width, innerWidth) : '100%', margin: 'auto', display: 'flex', flexDirection: 'column', background: '#0b1020'}}><Dashboard /></div>);
