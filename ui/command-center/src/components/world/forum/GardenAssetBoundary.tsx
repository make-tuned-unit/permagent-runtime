import { Component, type ReactNode } from 'react';
/** Canvas-safe; a failed detail asset leaves the independent walkable floor alive. */
export class GardenAssetBoundary extends Component<{children:ReactNode;onFailure:()=>void;fallback?:ReactNode},{failed:boolean}> {
  state={failed:false};
  static getDerivedStateFromError(){return{failed:true};}
  componentDidCatch(){this.props.onFailure();}
  render(){return this.state.failed?(this.props.fallback??null):this.props.children;}
}
