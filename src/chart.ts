// Pure SVG renderer for the 14-day stats bar chart. No dependencies.

import type { DayStat, Stats } from './types';

const WIDTH = 280; // viewBox width; scales to 100% via CSS
const HEIGHT = 80;
const PADDING_TOP = 4;
const PADDING_BOTTOM = 16; // room for day labels
const LABEL_Y = HEIGHT - 4;

function escapeAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function dayLabel(dayKey: string): string {
  // "YYYY-MM-DD" -> "DD"
  const parts = dayKey.split('-');
  return parts.length === 3 ? parts[2] : dayKey;
}

/** Renders the 14-day stats bar chart as an inline SVG string. Pure function, no DOM. */
export function renderStatsSvg(stats: Stats, dailyGoal: number): string {
  const days: DayStat[] = stats.days;
  const n = days.length;
  const chartHeight = HEIGHT - PADDING_TOP - PADDING_BOTTOM;
  const maxDrinks = Math.max(dailyGoal, ...days.map((d) => d.drinks), 1);

  const barGap = 2;
  const barWidth = n > 0 ? (WIDTH - barGap * (n - 1)) / n : WIDTH;

  const goalY = PADDING_TOP + chartHeight * (1 - Math.min(1, dailyGoal / maxDrinks));

  const bars = days
    .map((d, i) => {
      const x = i * (barWidth + barGap);
      const h = chartHeight * Math.min(1, d.drinks / maxDrinks);
      const y = PADDING_TOP + (chartHeight - h);
      const cls = d.goalMet ? 'bar bar-goal-met' : 'bar bar-below-goal';
      const avgGapText = d.avgGapMin === null ? 'n/a' : `${Math.round(d.avgGapMin)} min`;
      const title = `${d.dayKey}: ${d.drinks} drinks, avg gap ${avgGapText}, longest overdue ${Math.round(
        d.longestOverdueMin
      )} min`;
      const label = i % 2 === 0 ? `<text class="bar-label" x="${x + barWidth / 2}" y="${LABEL_Y}" text-anchor="middle">${escapeAttr(
        dayLabel(d.dayKey)
      )}</text>` : '';
      return `<g><rect class="${cls}" x="${x.toFixed(2)}" y="${y.toFixed(2)}" width="${barWidth.toFixed(
        2
      )}" height="${Math.max(0, h).toFixed(2)}"><title>${escapeAttr(title)}</title></rect>${label}</g>`;
    })
    .join('');

  const goalLine = `<line class="goal-line" x1="0" y1="${goalY.toFixed(2)}" x2="${WIDTH}" y2="${goalY.toFixed(
    2
  )}" stroke-dasharray="3,3" />`;

  return `<svg class="stats-chart" viewBox="0 0 ${WIDTH} ${HEIGHT}" width="100%" height="${HEIGHT}" xmlns="http://www.w3.org/2000/svg">${bars}${goalLine}</svg>`;
}
