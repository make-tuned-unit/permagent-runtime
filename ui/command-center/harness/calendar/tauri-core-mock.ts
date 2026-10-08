import { MOCK_EVENTS } from './events';

export async function invoke(cmd: string, args?: { start: string; end: string }): Promise<unknown> {
  if (cmd === 'calendar_authorization_status') return 'fullAccess';
  if (cmd === 'calendar_events_in_range' && args) {
    const s = Date.parse(args.start);
    const e = Date.parse(args.end);
    return MOCK_EVENTS.filter(x => x.startMs < e && x.endMs > s);
  }
  return undefined;
}
