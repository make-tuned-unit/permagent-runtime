/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot } from 'react-dom/client';
const app=vi.hoisted(()=>({navigate:vi.fn(()=>true),setActivePanel:vi.fn(),openGoalDetail:vi.fn(),openAgentSettings:vi.fn(),props:{} as Record<string,any>}));
vi.mock('../../../lib/store',()=>({navigateToTool:app.navigate,useCommandCenter:{getState:()=>app}}));
vi.mock('./ForumView',()=>({ForumView:(props:Record<string,unknown>)=>{app.props=props;return null;}}));
import { ForumAppView } from './ForumAppView';
(globalThis as Record<string,unknown>).IS_REACT_ACT_ENVIRONMENT=true;
it('connects World actions to the existing app navigation, goal details, skill library and agent settings',()=>{
 const host=document.createElement('div'),root=createRoot(host);
 act(()=>root.render(<ForumAppView visible={false}/>));
 expect(app.props.visible).toBe(false);
 act(()=>{app.props.onNavigate('memory');app.props.onNavigate('skills');app.props.onOpenGoal('project-1','goal-1');app.props.onManageAgent('git_steward');});
 expect(app.navigate).toHaveBeenCalledWith('memory');expect(app.setActivePanel).toHaveBeenCalledWith('skills');
 expect(app.openGoalDetail).toHaveBeenCalledWith('project-1','goal-1');expect(app.openAgentSettings).toHaveBeenCalledWith('git_steward');
 act(()=>root.unmount());
});
