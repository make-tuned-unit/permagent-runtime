/** App integration seam: use the existing store, navigation, details and settings.
 * The shipping World route is switched only when the integration gate is met.
 */
import { useState } from 'react';
import { navigateToTool, useCommandCenter } from '../../../lib/store';
import { ForumView } from './ForumView';
export function ForumAppView({ visible=true }: {visible?:boolean}) {
  const [navigationError,setNavigationError]=useState('');
  return <>{navigationError&&<p role="alert">{navigationError}</p>}<ForumView visible={visible} onNavigate={tool=>{
    setNavigationError('');
    if(tool==='skills')useCommandCenter.getState().setActivePanel('skills');
    else if(!navigateToTool(tool))setNavigationError('No workspace currently hosts this tool. Open it from the app navigation.');
  }} onOpenGoal={(projectId,id)=>useCommandCenter.getState().openGoalDetail(projectId,id)}
    onManageAgent={id=>useCommandCenter.getState().openAgentSettings(id)} /></>;
}
