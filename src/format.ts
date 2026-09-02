// Pure formatting helpers shared by widget and settings. Dependency-free.

import type { Tick, Settings } from './types';

/** Formats a Date as "HH:MM" (24h, zero-padded). */
export function formatHHMM(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** Parses "HH:MM" into minutes-since-midnight. Returns null if malformed. */
export function parseHHMMToMinutes(s: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(s);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h < 0 || h > 23 || min < 0 || min > 59) return null;
  return h * 60 + min;
}

/** Given today's activeStart "HH:MM", returns the next Date (today or tomorrow) that time occurs, relative to `now`. */
export function nextOccurrence(hhmm: string, now: Date): Date | null {
  const mins = parseHHMMToMinutes(hhmm);
  if (mins === null) return null;
  const result = new Date(now);
  result.setHours(Math.floor(mins / 60), mins % 60, 0, 0);
  if (result.getTime() <= now.getTime()) {
    result.setDate(result.getDate() + 1);
  }
  return result;
}

/** Builds the extra tooltip lines for mode/snooze state, in addition to the base "Last drink / Next / Today" line. */
export function modeTooltipLines(tick: Tick, settings: Settings, now: Date = new Date()): string[] {
  const lines: string[] = [];
  if (tick.mode === 'paused' && tick.pausedSince !== null) {
    lines.push(`Paused since ${formatHHMM(new Date(tick.pausedSince))}`);
  } else if (tick.mode === 'sleeping') {
    const until = nextOccurrence(settings.activeStart, now);
    if (until) {
      lines.push(`Sleeping until ${formatHHMM(until)}`);
    } else {
      lines.push('Sleeping');
    }
  }
  if (tick.snoozeMs > 0) {
    const mins = Math.round(tick.snoozeMs / 60000);
    lines.push(`Snoozed +${mins} min`);
  }
  return lines;
}
