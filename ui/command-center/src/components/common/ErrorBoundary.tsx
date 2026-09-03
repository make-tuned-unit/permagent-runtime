// ErrorBoundary — a single render throw must never white-screen the app
// (premium audit CRITICAL #1). Catches the subtree, shows an on-brand recovery
// card, and lets the user retry without losing the whole session. Per-surface
// boundaries (World, Brain) keep a crash in one heavy view from taking the
// shell down with it.

import { Component, type ErrorInfo, type ReactNode } from 'react';
import { radius, space, textSize } from '../../styles/tokens';

interface Props {
  /** Human label for what failed, e.g. "the World View". */
  surface?: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // The only trace we get — the daemon has no client error sink yet (#327).
    console.error(`[error-boundary${this.props.surface ? ` ${this.props.surface}` : ''}]`, error, info.componentStack);
  }

  private reset = () => this.setState({ error: null });

  render() {
    if (!this.state.error) return this.props.children;
    const what = this.props.surface ?? 'This view';
    return (
      <div
        role="alert"
        style={{
          height: '100%',
          minHeight: 200,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 14,
          padding: 32,
          textAlign: 'center',
          fontFamily: 'Inter, system-ui, sans-serif',
          color: '#8A94A6',
          background: 'radial-gradient(ellipse 80% 60% at 50% 40%, rgba(0,213,255,0.04) 0%, #0A0E1A 70%)',
        }}
      >
        <div style={{ fontSize: 26 }}>◇</div>
        <div style={{ fontSize: textSize.body, fontWeight: 600, color: '#E8E4DD' }}>
          {what} hit a snag
        </div>
        <div style={{ fontSize: textSize.caption, maxWidth: 380, lineHeight: 1.5 }}>
          Something in this view stopped working — the rest of the app is fine.
          Try again, or switch tabs and come back.
        </div>
        <button
          onClick={this.reset}
          style={{
            marginTop: space.xs,
            padding: `${space.md}px 18px`,
            borderRadius: radius.md,
            border: '1px solid rgba(0,213,255,0.4)',
            background: 'rgba(0,213,255,0.1)',
            color: '#00D5FF',
            fontSize: textSize.small,
            fontWeight: 600,
            cursor: 'pointer',
          }}
        >
          Try again
        </button>
      </div>
    );
  }
}
