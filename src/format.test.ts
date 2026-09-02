// Tiny dependency-free test script. Run with `npx tsx src/format.test.ts`.

import { formatHHMM, parseHHMMToMinutes, nextOccurrence, modeTooltipLines } from './format';
import type { Tick, Settings } from './types';

let failures = 0;

function assert(cond: boolean, msg: string) {
  if (!cond) {
    failures++;
    console.error(`FAIL: ${msg}`);
  } else {
    console.log(`ok: ${msg}`);
  }
}

// --- formatHHMM ---
assert(formatHHMM(new Date(2026, 0, 1, 8, 5)) === '08:05', 'formatHHMM pads single digits');
assert(formatHHMM(new Date(2026, 0, 1, 22, 0)) === '22:00', 'formatHHMM formats 22:00');

// --- parseHHMMToMinutes ---
assert(parseHHMMToMinutes('08:00') === 480, 'parseHHMMToMinutes 08:00 = 480');
assert(parseHHMMToMinutes('23:59') === 1439, 'parseHHMMToMinutes 23:59 = 1439');
assert(parseHHMMToMinutes('bogus') === null, 'parseHHMMToMinutes rejects garbage');
assert(parseHHMMToMinutes('24:00') === null, 'parseHHMMToMinutes rejects hour 24');

// --- nextOccurrence ---
const now1 = new Date(2026, 0, 1, 7, 0);
const occ1 = nextOccurrence('08:00', now1);
assert(occ1 !== null && occ1.getDate() === 1 && occ1.getHours() === 8, 'nextOccurrence same day when in future');

const now2 = new Date(2026, 0, 1, 9, 0);
const occ2 = nextOccurrence('08:00', now2);
assert(occ2 !== null && occ2.getDate() === 2 && occ2.getHours() === 8, 'nextOccurrence rolls to tomorrow when already past');

// --- modeTooltipLines ---
function baseTick(overrides: Partial<Tick>): Tick {
  return {
    fill: 1,
    zone: 'fresh',
    remainingMs: 0,
    overdueMs: 0,
    todayCount: 0,
    intervalMs: 45 * 60000,
    lastDrinkTs: Date.now(),
    mode: 'active',
    quiet: false,
    snoozeMs: 0,
    pausedSince: null,
    ...overrides,
  };
}

function baseSettings(overrides: Partial<Settings>): Settings {
  return {
    intervalMin: 45,
    opacity: 0.9,
    showText: true,
    showCount: true,
    activeStart: '08:00',
    activeEnd: '22:00',
    quietStart: '22:00',
    quietEnd: '08:00',
    dailyGoal: 8,
    alwaysOnTop: true,
    clickThrough: false,
    autostart: false,
    hotkeyEnabled: true,
    hotkey: 'Ctrl+Alt+W',
    toastEnabled: true,
    nudgeEveryMin: 10,
    nudgeMax: 3,
    soundEnabled: false,
    soundVolume: 0.5,
    ...overrides,
  };
}

const pausedTick = baseTick({ mode: 'paused', pausedSince: new Date(2026, 0, 1, 14, 5).getTime() });
const pausedLines = modeTooltipLines(pausedTick, baseSettings({}));
assert(pausedLines.includes('Paused since 14:05'), 'modeTooltipLines shows paused-since time');

const sleepingTick = baseTick({ mode: 'sleeping' });
const sleepingLines = modeTooltipLines(sleepingTick, baseSettings({ activeStart: '08:00' }), new Date(2026, 0, 1, 23, 0));
assert(sleepingLines.some((l) => l.startsWith('Sleeping until')), 'modeTooltipLines shows sleeping-until');

const snoozedTick = baseTick({ snoozeMs: 10 * 60000 });
const snoozedLines = modeTooltipLines(snoozedTick, baseSettings({}));
assert(snoozedLines.includes('Snoozed +10 min'), 'modeTooltipLines shows snooze amount');

const plainTick = baseTick({});
const plainLines = modeTooltipLines(plainTick, baseSettings({}));
assert(plainLines.length === 0, 'modeTooltipLines empty for plain active/no-snooze tick');

if (failures > 0) {
  console.error(`\n${failures} test(s) failed.`);
  process.exit(1);
} else {
  console.log('\nAll tests passed.');
}
