// Real production components and API calls, backed by the isolated HTTP test.
import React from 'react';
import {createRoot} from 'react-dom/client';
import '../src/index.css';
import {ProjectsView} from '../src/components/projects/ProjectsView';
import {PeopleView} from '../src/components/people/PeopleView';
import {AutomateView} from '../src/components/automate/AutomateView';
import {SessionsList} from '../src/components/sessions/SessionsList';
import {useCommandCenter} from '../src/lib/store';
import {setTheme, setReduceMotion} from '../src/styles/tokens';

const response = await fetch('/__ux_review');
if (!response.ok) throw new Error('Isolated staging is not ready');
const state = await response.json();
localStorage.setItem('permagent-daemon-token', state.token);
localStorage.setItem('permagent-people-mode', 'graph');
useCommandCenter.setState({currentProjectId: state.projectId, pendingProjectNavigation: state.projectId});
const params = new URLSearchParams(location.search);
setTheme(params.get('theme') === 'silver' ? 'silver' : 'dark');
setReduceMotion(true);
const page = params.get('page') || 'projects';
const surfaces: Record<string, React.ReactNode> = {
  projects: <ProjectsView />, people: <PeopleView />, automate: <AutomateView />, history: <SessionsList />,
};
createRoot(document.getElementById('root')!).render(<div style={{height: '100vh', maxWidth: '100%', width: Number(params.get('width')) || '100%', margin: 'auto'}}>
  {surfaces[page] || surfaces.projects}
</div>);
