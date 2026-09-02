// Tiny dependency-free test script. Run with `npx tsx src/chart.test.ts`.

import { renderStatsSvg } from './chart';
import type { DayStat, Stats } from './types';

let failures = 0;

function assert(cond: boolean, msg: string) {
  if (!cond) {
    failures++;
    console.error(`FAIL: ${msg}`);
  } else {
    console.log(`ok: ${msg}`);
  }
}

function makeDays(drinksPerDay: number[], goal: number): DayStat[] {
  const base = new Date('2026-08-20T00:00:00Z');
  return drinksPerDay.map((drinks, i) => {
    const d = new Date(base.getTime() + i * 86400000);
    const dayKey = d.toISOString().substring(0, 10);
    return {
      dayKey,
      drinks,
      avgGapMin: drinks >= 2 ? 60 : null,
      longestOverdueMin: drinks === 0 ? 0 : 5,
      goalMet: drinks >= goal,
    };
  });
}

const goal = 8;
const drinks = [0, 3, 8, 9, 5, 8, 10, 2, 8, 7, 8, 8, 0, 8];
const days = makeDays(drinks, goal);
const stats: Stats = {
  days,
  streak: 2,
  bestStreak: 5,
  totalDrinks: drinks.reduce((a, b) => a + b, 0),
};

const svg = renderStatsSvg(stats, goal);

// --- bar count ---
const rectMatches = svg.match(/<rect /g) || [];
assert(rectMatches.length === 14, `renders 14 bars (got ${rectMatches.length})`);

// --- goal line present ---
assert(svg.includes('goal-line'), 'includes a goal line element');
assert(svg.includes('stroke-dasharray'), 'goal line is dashed');

// --- class assignment ---
const metCount = (svg.match(/bar-goal-met/g) || []).length;
const belowCount = (svg.match(/bar-below-goal/g) || []).length;
const expectedMet = drinks.filter((d) => d >= goal).length;
const expectedBelow = drinks.length - expectedMet;
assert(metCount === expectedMet, `bar-goal-met count matches (${metCount} === ${expectedMet})`);
assert(belowCount === expectedBelow, `bar-below-goal count matches (${belowCount} === ${expectedBelow})`);

// --- valid svg root ---
assert(svg.startsWith('<svg'), 'output starts with <svg');
assert(svg.includes('viewBox'), 'has a viewBox');

// --- day labels present for every other day ---
const labelMatches = svg.match(/bar-label/g) || [];
assert(labelMatches.length === 7, `renders label for every other day (got ${labelMatches.length})`);

// --- tooltip titles present ---
assert((svg.match(/<title>/g) || []).length === 14, 'each bar has a title tooltip');
assert(svg.includes('drinks, avg gap'), 'title text includes avg gap phrase');

// --- handles empty days array without throwing ---
const emptyStats: Stats = { days: [], streak: 0, bestStreak: 0, totalDrinks: 0 };
let threw = false;
try {
  renderStatsSvg(emptyStats, goal);
} catch {
  threw = true;
}
assert(!threw, 'does not throw on empty days array');

if (failures > 0) {
  console.error(`\n${failures} test(s) failed.`);
  process.exit(1);
} else {
  console.log('\nAll tests passed.');
}
