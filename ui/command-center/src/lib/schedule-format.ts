/** Convert a 5-field cron expression to natural English. */
export function cronToEnglish(cron: string): string {
  const parts = cron.trim().split(/\s+/);
  if (parts.length < 5) return cron;

  const [min, hour, dom, month, dow] = parts;

  // Exact presets
  const presets: Record<string, string> = {
    '0 8 * * 1-5': 'Every weekday morning at 8 AM',
    '0 9 * * 1-5': 'Every weekday morning at 9 AM',
    '0 19 * * 0': 'Every Sunday at 7 PM',
    '0 8 * * *': 'Every morning at 8 AM',
    '0 9 * * 1': 'Every Monday at 9 AM',
    '0 0 * * *': 'Every day at midnight',
    '0 * * * *': 'Every hour',
    '0 0 1 * *': 'First day of every month',
  };

  const key = parts.slice(0, 5).join(' ');
  if (presets[key]) return presets[key];

  // Pattern matching
  const minMatch = min.match(/^\*\/(\d+)$/);
  if (minMatch && hour === '*' && dom === '*' && month === '*' && dow === '*') {
    return `Every ${minMatch[1]} minutes`;
  }

  const hourMatch = hour.match(/^\*\/(\d+)$/);
  if (min === '0' && hourMatch && dom === '*' && month === '*' && dow === '*') {
    return `Every ${hourMatch[1]} hours`;
  }

  // Specific time, every day
  if (min.match(/^\d+$/) && hour.match(/^\d+$/) && dom === '*' && month === '*') {
    const h = parseInt(hour);
    const m = parseInt(min);
    const timeStr = formatTime(h, m);

    if (dow === '*') return `Every day at ${timeStr}`;
    if (dow === '1-5') return `Every weekday at ${timeStr}`;
    if (dow === '0,6') return `Weekends at ${timeStr}`;

    const dayName = dayOfWeekName(dow);
    if (dayName) return `Every ${dayName} at ${timeStr}`;

    return `At ${timeStr} on days ${dow}`;
  }

  // Specific day of month
  if (min.match(/^\d+$/) && hour.match(/^\d+$/) && dom.match(/^\d+$/) && month === '*' && dow === '*') {
    const h = parseInt(hour);
    const m = parseInt(min);
    const d = parseInt(dom);
    return `Day ${d} of every month at ${formatTime(h, m)}`;
  }

  // Fallback: structured description
  return `Cron: ${cron}`;
}

function formatTime(h: number, m: number): string {
  const period = h >= 12 ? 'PM' : 'AM';
  const displayH = h === 0 ? 12 : h > 12 ? h - 12 : h;
  if (m === 0) return `${displayH} ${period}`;
  return `${displayH}:${m.toString().padStart(2, '0')} ${period}`;
}

function dayOfWeekName(dow: string): string | null {
  const days: Record<string, string> = {
    '0': 'Sunday', '1': 'Monday', '2': 'Tuesday', '3': 'Wednesday',
    '4': 'Thursday', '5': 'Friday', '6': 'Saturday', '7': 'Sunday',
  };
  return days[dow] || null;
}
