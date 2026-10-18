// Synthetic visual review: intercepted reads, denied writes, no daemon proxy.
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../src/index.css';
import { ProjectsView } from '../src/components/projects/ProjectsView';
import { PeopleView } from '../src/components/people/PeopleView';
import { AutomateView } from '../src/components/automate/AutomateView';
import { FinanceView } from '../src/components/finance/FinanceView';
import { GrowAnalytics } from '../src/components/grow/GrowAnalytics';
import { GrowActions } from '../src/components/grow/GrowActions';
import { GrowResults } from '../src/components/grow/GrowResults';
import { StrategyLens } from '../src/components/grow/StrategyLens';
import { SettingsView } from '../src/components/settings/SettingsView';
import { BrainView } from '../src/components/brain/BrainView';
import { SessionsList } from '../src/components/sessions/SessionsList';
import { useCommandCenter } from '../src/lib/store';
import { getThemedColors, setTheme, setReduceMotion } from '../src/styles/tokens';
const params = new URLSearchParams(location.search);
const page = params.get('page') || 'people';
const now = new Date().toISOString();
const empty = params.get('state') === 'empty';
const project = { id: 'garden', slug: 'garden', name: 'Garden journal', description: 'Track seasonal growth and planting plans.', status: 'active', rootPath: '/synthetic/garden', siteUrl: null, repoUrl: null, tags: [], metadataJson: {}, createdAt: now, updatedAt: now, lastOpenedAt: now };
const projects = empty ? [] : params.has('dense') ? [
  {...project, id: '00000000-0000-0000-0000-000000000001', slug: 'personal', name: 'Personal', description: 'Default project for unscoped activity.', tags: []},
  ...Array.from({length: 23}, (_, i) => ({...project, id: `review-project-${i}`, slug: `review-project-${i}`,
    name: ['Garden journal', 'Neighborhood workshop', 'Community research and local history archive', 'Seed library'][i % 4] + ` ${i + 1}`,
    description: i % 5 === 0 ? '' : 'A shared project with notes, upcoming work, and a growing collection of useful resources.',
    status: i < 20 ? 'active' : i < 22 ? 'paused' : 'archived',
    tags: i % 3 === 0 ? [] : ['community', 'planning', 'local-history', 'research', 'accessibility'],
  })),
] : [project, {...project, id: 'shelved', slug: 'shelved', name: 'Winter plans', status: 'paused'}];
const analytics = {enabled: true, receiving: !empty, periodDays: 30, pageviews: empty ? 0 : 275,
  deviceSignatures: empty ? 0 : 153, signedUpUsers: null, sessions: empty ? 0 : 217,
  eventsLast5m: empty ? 0 : 30, botsExcluded: 29, includingBots: false, lastDrainAt: now,
  bounceRate: empty ? null : 0.87, pagesPerSession: empty ? null : 1.3,
  byDay: empty ? [] : Array.from({length: 30}, (_, i) => ({day: new Date(Date.now() - (29-i)*86400000).toISOString().slice(0,10), pageviews: i < 17 ? 0 : [65, 41, 19, 17, 12, 12, 17, 20, 23, 17, 13, 7, 12][i-17], visitors: i < 17 ? 0 : 8})),
  topPages: [{name: '/', count: 120}, {name: '/guides', count: 104}, {name: '/download', count: 13}],
  topReferrers: [{name: 'example.test', count: 91}], topEvents: [{name: 'download', count: 29}, {name: 'checkout_started', count: 14}],
  topSources: [{name: 'direct / none', count: 129}], topCampaigns: [], topEntryPages: [{name: '/', count: 91}],
  period: {days: 30, start: new Date(Date.now()-30*86400000).toISOString(), end: now}};
const people = empty ? [] : ['Sam Rivera', 'Alex Chen', 'Morgan Lee', 'Taylor Brooks', 'Casey Patel', 'Riley Park'].map((display_name, i) => ({entity_uuid: `person-${i}`, canonical_id: `person:${i}`, display_name, role: i === 0 ? 'Landscape designer' : 'Community gardener', company: 'Community garden', email: 'example@example.test', phone: null, notes: 'Planning the spring planting workshop together.', how_met: 'Met at the garden workshop', photo_url: null, last_contact_at: now, created_at: now, updated_at: now, projects: [{ project_id: 'garden', project_name: 'Garden journal' }]}));
const jobs = empty ? [] : Array.from({length: 5}, (_, i) => ({id: `job-${i}`, source: 'fixture', display_name: ['Garden digest', 'Review planting notes', 'Summarize project activity'][i % 3], cron: '0 9 * * 1-5', currently_running: i === 0, paused: false, last_status: 'ok', last_run: now, next_run: now, process_start_time: now, description: 'A synthetic schedule for visual review'}));
const finance = { polybot: {found: false, stale: false, paused: true, credentialsReady: false}, polybotEnabled: false, pickerEnabled: false,
  holdings: {source: 'ledger', openCount: 0, netUnrealized: 0, netRealized: 0, netPnl: 0, trend: [], rows: []}, watchlist: [], notes: [], positions: [],
  picker: {reachable: false, baseUrl: '', scanInProgress: false, paused: false, stale: false}, pickerUniverse: [], picks: [], sellSignals: [], rsiThreshold: 74, dailyPick: null,
  household: {recent: [], forecast: {daysUsed: 0, spend90d: 0, runRate30d: 0, runRate90d: 0, byCategory: [], recurring: [], method: 'trailing run-rate'}} };
const actions = empty ? [] : Array.from({length: 7}, (_, i) => ({title: ['Make planting dates easier to find', 'Improve the seed library search', 'Explain seasonal reminders'][i % 3], category: 'aeo', evidence: 'Synthetic review data: readers visited the planting guide.', recommendation: 'Make the next useful action clear and explain its context.', steps: ['Inspect the existing guide', 'Add accessible navigation', 'Verify the result'], artifactKind: 'none', artifact: null, impact: 'medium', confidence: 'medium', identity: {id: `action-${i}`, status: 'suggested', targetMetric: null, targetDir: null, outcomes: []}}));
localStorage.setItem('permagent-daemon-token', 'synthetic-fixture-not-a-credential');
localStorage.setItem('permagent-people-mode', 'list'); // Reproduce an old saved List choice.
window.fetch = async (input, init) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href);
  const path = url.pathname;
  const method = init?.method || 'GET';
  const response = (data: unknown, status = 200) => new Response(JSON.stringify(data), {status, headers: {'Content-Type':'application/json'}});
  // These POSTs are modeled read/import state only; nothing leaves this harness.
  if (path === '/config/read') return response(null);
  if (path === '/api/people/calendar/import') return response({imported: 0});
  if (method !== 'GET') return response({error: 'Writes are disabled in this review fixture'}, 405);
  if (params.get('state') === 'error') return response({error: 'Synthetic unavailable state'}, 503);
  if (path === '/api/projects') return response(projects);
  if (path.endsWith('/columns')) return response([{id: 'todo', stateBinding: null}]);
  if (path.endsWith('/cards')) return response(Array.from({length: 3}, (_, i) => ({id: `card-${i}`, columnId: 'todo', cardType: i === 0 ? 'goal' : 'task', archivedAt: null, updatedAt: now})));
  if (path === '/api/people' || path === '/api/people/directory') return response(people);
  if (path.endsWith('/projects') && path.includes('/people/')) return response([{ project_id: project.id, project_name: project.name, project_role: 'Collaborator', associated_at: now }]);
  if (/\/(relationships|activity|meetings|notes|memories|documents)$/.test(path)) return response([]);
  if (path === '/api/finance') return response(finance);
  if (path === '/schedule/list') return response({jobs});
  if (path === '/api/job-health') return response({jobs: jobs.map(j => ({id: j.id, label: j.display_name, outcome: 'ok', cadence: 'weekdays'}))});
  if (path === '/api/runs') return response({runs: []});
  if (path.startsWith('/schedule/sessions_recent')) return response({sessions: {}});
  if (path === '/config/extensions') return response({extensions: [], warnings: []});
  if (path.endsWith('/growth-inbox')) return response({moves: [], wins: [], signal: {posts: 0, shipped: 0, activeGoals: 0, daysSinceLastPost: null}});
  if (path.endsWith('/analytics/connection')) return response({connected: false, provider: null, baseUrl: null, siteId: null, hasApiKey: false});
  if (path.endsWith('/analytics/first_party/stats')) return response(analytics);
  if (path.endsWith('/analytics/first_party')) return response({enabled: true, receiving: !empty, siteKey: 'synthetic', ingestBase: 'https://example.test', ingestUrl: null, snippet: 'Synthetic installation snippet', agentPrompt: 'Synthetic installation instructions', drainUrl: 'https://example.test/drain', drainSecret: null, cursor: null, lastDrainAt: now, lastError: null});
  if (path === '/api/security/sovereignty') return response({enabled: false, capturePrompts: false, localProviderAvailable: true});
  if (path === '/api/security/egress-log' || path === '/api/security/auth-log') return response([]);
  if (path.includes('/growth-actions')) return response({actions, archived: [], dismissed: [], tracking: [], generating: false, generatedAt: now, periodDays: 30});
  if (path === '/api/growth-results') return response({project: null, fleet: {projects: 0, helped: 0, hindered: 0, noEffect: 0, inconclusive: 0, categories: [], recent: [], trend: [], byProject: []}});
  if (path === '/api/brain/graph') return response({self: {name: 'You', id: 'self'}, entities: people.map(p => ({id: p.entity_uuid, type: 'person', name: p.display_name, note: p.notes})), edges: [], memories: []});
  if (path === '/api/agent/identity') return response({first_name: 'Aria', display_name: 'Aria', traits: [], tone: 'warm', opening_greeting: '', voice_id: null});
  if (path === '/api/agents/roster') return response({workers: [], dispatch_roster: [], capabilities: []});
  if (path === '/api/sessions' || path === '/sessions') return response({sessions: []});
  if (path === '/sse-token') return response({token: 'fixture'});
  return response({error: 'No synthetic fixture for this read'}, 404);
};
window.WebSocket = class {close() {} send() {}} as unknown as typeof WebSocket;
useCommandCenter.setState({agentName: 'Aria', currentProjectId: null, loadSkills: async () => {}, loadProposals: async () => {}, skills: [], proposals: [], currentModel: 'fixture', loadProviders: async () => {}, pendingSettingsSection: params.get('section')});
setTheme(params.get('theme') === 'silver' ? 'silver' : 'dark');
setReduceMotion(params.has('motion') ? false : true);
const colors = getThemedColors();
const surfaces: Record<string, React.ReactNode> = {
 projects: <ProjectsView />, people: <PeopleView />, automate: <AutomateView />, finance: <FinanceView />,
 analytics: <GrowAnalytics project={project} posts={[]} colors={colors} />, actions: <GrowActions project={project} colors={colors} />, results: <GrowResults project={project} colors={colors} />,
 strategy: <StrategyLens active={project} colors={colors} send={() => {}} agentName="Aria" />,
 settings: <SettingsView />, brain: <BrainView />, history: <SessionsList />,
};
createRoot(document.getElementById('root')!).render(<div style={{width: Number(params.get('width')) || '100%', maxWidth: '100%', height: Number(params.get('height')) || 720, margin: 'auto', padding: ['actions','results','strategy'].includes(page) ? 24 : 0, overflow: 'auto'}}>{surfaces[page] || surfaces.people}</div>);
