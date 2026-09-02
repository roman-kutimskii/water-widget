// Tiny dependency-free test script. Run with `npx tsx src/color.test.ts`
// or `node --experimental-strip-types src/color.test.ts`.

import { fillColor, textColorOn, formatRemaining } from './color';
import type { Tick } from './types';

let failures = 0;

function assert(cond: boolean, msg: string) {
  if (!cond) {
    failures++;
    console.error(`FAIL: ${msg}`);
  } else {
    console.log(`ok: ${msg}`);
  }
}

function hexClose(a: string, b: string, tol = 2): boolean {
  const pa = [1, 3, 5].map((i) => parseInt(a.substring(i, i + 2), 16));
  const pb = [1, 3, 5].map((i) => parseInt(b.substring(i, i + 2), 16));
  return pa.every((v, i) => Math.abs(v - pb[i]) <= tol);
}

// --- fillColor: exact stops should round-trip closely ---
assert(hexClose(fillColor(1.0), '#3B82F6'), 'fillColor(1.0) matches blue stop');
assert(hexClose(fillColor(0.6), '#22C55E'), 'fillColor(0.6) matches green stop');
assert(hexClose(fillColor(0.45), '#EAB308'), 'fillColor(0.45) matches yellow stop');
assert(hexClose(fillColor(0.3), '#F97316'), 'fillColor(0.3) matches orange stop');
assert(hexClose(fillColor(0.0), '#EF4444'), 'fillColor(0.0) matches red stop');

// Midpoint should be a valid hex color, not equal to either endpoint.
const mid = fillColor(0.75);
assert(/^#[0-9A-Fa-f]{6}$/.test(mid), 'fillColor(0.75) returns valid hex');
assert(!hexClose(mid, '#3B82F6', 0) , 'fillColor(0.75) differs from pure blue stop');

// Clamping out-of-range input.
assert(fillColor(-1) === fillColor(0), 'fillColor clamps below 0');
assert(fillColor(2) === fillColor(1), 'fillColor clamps above 1');

// --- textColorOn ---
assert(textColorOn('#3B82F6') === '#FFFFFF' || textColorOn('#3B82F6') === '#111111', 'textColorOn returns a valid choice for blue');
assert(textColorOn('#000000') === '#FFFFFF', 'textColorOn(black) picks white');
assert(textColorOn('#FFFFFF') === '#111111', 'textColorOn(white) picks near-black');

// --- formatRemaining ---
function tick(overrides: Partial<Tick>): Tick {
  return {
    fill: 1,
    zone: 'fresh',
    remainingMs: 0,
    overdueMs: 0,
    todayCount: 0,
    intervalMs: 45 * 60000,
    lastDrinkTs: Date.now(),
    ...overrides,
  };
}

assert(
  formatRemaining(tick({ zone: 'fresh', remainingMs: 42 * 60000 })) === '42 min',
  'formatRemaining fresh 42 min'
);
assert(
  formatRemaining(tick({ zone: 'urgent', remainingMs: 8 * 60000 })) === '8 min',
  'formatRemaining urgent 8 min'
);
assert(
  formatRemaining(tick({ zone: 'overdue', overdueMs: 12 * 60000, remainingMs: 0 })) === '+12 min',
  'formatRemaining overdue +12 min'
);
assert(
  formatRemaining(tick({ zone: 'overdue', overdueMs: 30 * 1000, remainingMs: 0 })) === '<1 min',
  'formatRemaining overdue <1 min'
);
assert(
  formatRemaining(tick({ zone: 'fresh', remainingMs: 30 * 1000 })) === '<1 min',
  'formatRemaining fresh <1 min (sub-minute remaining)'
);

if (failures > 0) {
  console.error(`\n${failures} test(s) failed.`);
  process.exit(1);
} else {
  console.log('\nAll tests passed.');
}
