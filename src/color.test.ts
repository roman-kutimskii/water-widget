// Tiny dependency-free test script. Run with `npx tsx src/color.test.ts`
// or `node --experimental-strip-types src/color.test.ts`.

import { fillColor, overdueColor, textColorOn, formatRemaining } from './color';
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

// --- fillColor presets ---
assert(hexClose(fillColor(1.0, 'default'), '#3B82F6'), 'default preset fillColor(1.0) matches blue');
assert(hexClose(fillColor(1.0, 'colorblind'), '#3B82F6'), 'colorblind fillColor(1.0) matches blue stop');
assert(hexClose(fillColor(0.5, 'colorblind'), '#8B5CF6'), 'colorblind fillColor(0.5) matches purple stop');
assert(hexClose(fillColor(0.0, 'colorblind'), '#D946EF'), 'colorblind fillColor(0.0) matches magenta stop');
assert(fillColor(1.0, 'mono') === '#9CA3AF', 'mono fillColor(1.0) is flat grey');
assert(fillColor(0.2, 'mono') === '#9CA3AF', 'mono fillColor(0.2) is flat grey');
assert(fillColor(0.0, 'mono') === fillColor(1.0, 'mono'), 'mono fillColor is constant regardless of fill');

// Colorblind preset: within each half of the range (1.0->0.5 and 0.5->0.0, the
// segments actually interpolated in OKLCH) lightness should move monotonically
// between the segment's own endpoints, per CONTRACT.md's per-stop OKLCH scheme.
// (Note: across the full 1.0->0.0 span the *contract-fixed* hex endpoints
// themselves are not monotonic in luminance — #D946EF is lighter than
// #3B82F6 by every standard luminance metric — so we verify monotonicity
// within each interpolated segment instead, which is what the OKLCH
// interpolation actually guarantees.)
function oklchLightness(hex: string): number {
  const to01 = (v: number) => v / 255;
  const lin = (v: number) => (v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4));
  const r = lin(to01(parseInt(hex.substring(1, 3), 16)));
  const g = lin(to01(parseInt(hex.substring(3, 5), 16)));
  const b = lin(to01(parseInt(hex.substring(5, 7), 16)));
  const l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
  const m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
  const s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
  const l_ = Math.cbrt(l);
  const m_ = Math.cbrt(m);
  const s_ = Math.cbrt(s);
  return 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
}
function isMonotonic(samples: number[]): boolean {
  let increasing = true;
  let decreasing = true;
  for (let i = 1; i < samples.length; i++) {
    if (samples[i] > samples[i - 1] + 1e-9) decreasing = false;
    if (samples[i] < samples[i - 1] - 1e-9) increasing = false;
  }
  return increasing || decreasing;
}
{
  const upperHalf = [1.0, 0.9, 0.8, 0.7, 0.6, 0.5].map((f) => oklchLightness(fillColor(f, 'colorblind')));
  const lowerHalf = [0.5, 0.4, 0.3, 0.2, 0.1, 0.0].map((f) => oklchLightness(fillColor(f, 'colorblind')));
  assert(isMonotonic(upperHalf), 'colorblind lightness is monotonic across the 1.0->0.5 segment');
  assert(isMonotonic(lowerHalf), 'colorblind lightness is monotonic across the 0.5->0.0 segment');
}

// --- overdueColor ---
assert(overdueColor('default') === '#EF4444', 'overdueColor default is red');
assert(overdueColor('colorblind') === '#D946EF', 'overdueColor colorblind is magenta');
assert(overdueColor('mono') === '#9CA3AF', 'overdueColor mono is grey');

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
