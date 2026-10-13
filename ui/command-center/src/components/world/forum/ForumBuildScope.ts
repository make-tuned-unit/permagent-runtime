/** Releases partial procedural builds when their Canvas is discarded. */
export class ForumBuildScope {
  private cleanup: (() => void)[] = [];
  own<T extends { dispose(): void }>(value: T): T { this.cleanup.push(() => value.dispose()); return value; }
  defer(cleanup: () => void) { this.cleanup.push(cleanup); }
  dispose() { for (const cleanup of this.cleanup.reverse()) cleanup(); this.cleanup = []; }
}
