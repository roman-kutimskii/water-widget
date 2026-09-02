import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { fillColor, formatRemaining } from './color';
import { modeTooltipLines } from './format';
import { playDropSound } from './sound';
import type { Tick, Settings, Nudge } from './types';

const pillEl = document.getElementById('pill') as HTMLDivElement;
const fillEl = document.getElementById('fill') as HTMLDivElement;
const textEl = document.getElementById('text') as HTMLDivElement;
const htmlEl = document.documentElement;

let currentTick: Tick | null = null;
let currentSettings: Settings | null = null;

function applyGlobalSettingsClasses(settings: Settings | null) {
  const s = settings;
  const layout = s?.layout ?? 'horizontal';
  htmlEl.classList.remove('layout-horizontal', 'layout-vertical', 'layout-compact');
  htmlEl.classList.add(`layout-${layout}`);

  const rm = s?.reducedMotion ?? 'system';
  htmlEl.classList.remove('rm-on', 'rm-off');
  if (rm === 'on') htmlEl.classList.add('rm-on');
  else if (rm === 'off') htmlEl.classList.add('rm-off');

  const preset = s?.colorPreset ?? 'default';
  htmlEl.classList.remove('preset-default', 'preset-colorblind', 'preset-mono');
  htmlEl.classList.add(`preset-${preset}`);

  const scale = s?.scale ?? 1.0;
  htmlEl.style.fontSize = `${12 * scale}px`;
}

function render() {
  if (!currentTick) return;
  const tick = currentTick;
  const settings = currentSettings;
  const preset = settings?.colorPreset ?? 'default';
  const layout = settings?.layout ?? 'horizontal';

  pillEl.classList.toggle('overdue', tick.zone === 'overdue' && tick.mode === 'active');
  pillEl.classList.toggle('urgent', tick.zone === 'urgent' && tick.mode === 'active');
  pillEl.classList.toggle('paused', tick.mode === 'paused');
  pillEl.classList.toggle('sleeping', tick.mode === 'sleeping');
  pillEl.classList.toggle('quiet', tick.quiet);
  pillEl.classList.toggle('click-through', !!settings?.clickThrough);

  const isVertical = layout === 'vertical';

  if (tick.mode === 'paused' || tick.mode === 'sleeping') {
    if (isVertical) {
      fillEl.style.height = '100%';
    } else {
      fillEl.style.width = '100%';
    }
    fillEl.style.backgroundColor = '#9CA3AF';
  } else {
    const color = fillColor(tick.fill, preset);
    if (isVertical) {
      fillEl.style.height = `${tick.fill * 100}%`;
      fillEl.style.width = '100%';
    } else {
      fillEl.style.width = `${tick.fill * 100}%`;
    }
    fillEl.style.backgroundColor = color;
  }

  const baseOpacity = settings ? settings.opacity : 0.9;
  pillEl.style.opacity = String(tick.quiet ? baseOpacity * 0.4 : baseOpacity);

  const showText = settings ? settings.showText : true;
  const showCount = settings ? settings.showCount : true;
  const dailyGoal = settings ? settings.dailyGoal : 8;

  if (showText) {
    let label: string;
    if (tick.mode === 'paused') {
      label = 'Paused';
    } else if (tick.mode === 'sleeping') {
      label = 'zzz';
    } else {
      label = formatRemaining(tick);
    }
    if (showCount) {
      label += ` · ${tick.todayCount} / ${dailyGoal}`;
    }
    textEl.textContent = label;
    textEl.classList.remove('hidden');
  } else {
    textEl.classList.add('hidden');
  }

  pillEl.title = tooltipFor(tick, settings);
}

function tooltipFor(tick: Tick, settings: Settings | null): string {
  const last = new Date(tick.lastDrinkTs);
  const next = new Date(tick.lastDrinkTs + tick.intervalMs);
  const fmt = (d: Date) =>
    `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  let base = `Last drink ${fmt(last)} · Next ${fmt(next)} · Today ${tick.todayCount}`;
  if (settings) {
    const extra = modeTooltipLines(tick, settings);
    for (const line of extra) {
      base += ` · ${line}`;
    }
  }
  return base;
}

async function init() {
  const [tick, settings] = await Promise.all([
    invoke<Tick>('get_tick'),
    invoke<Settings>('get_settings'),
  ]);
  currentTick = tick;
  currentSettings = settings;
  applyGlobalSettingsClasses(currentSettings);
  render();

  await listen<Tick>('tick', (event) => {
    currentTick = event.payload;
    render();
  });

  await listen<Settings>('settings-changed', (event) => {
    currentSettings = event.payload;
    applyGlobalSettingsClasses(currentSettings);
    render();
  });

  await listen<Nudge>('nudge', (event) => {
    onNudge(event.payload);
  });
}

function onNudge(_nudge: Nudge): void {
  // 300 ms wobble, respecting prefers-reduced-motion (handled purely in CSS).
  pillEl.classList.remove('wobble');
  // Force reflow so the animation can restart if already applied.
  void pillEl.offsetWidth;
  pillEl.classList.add('wobble');
  window.setTimeout(() => pillEl.classList.remove('wobble'), 320);

  if (currentSettings?.soundEnabled) {
    try {
      playDropSound(currentSettings.soundVolume);
    } catch (err) {
      console.error('nudge sound failed', err);
    }
  }
}

// --- Click vs drag: we do NOT use data-tauri-drag-region because on Windows the
// native move loop swallows mouseup, so clicks would never register. Instead we
// start the native drag ourselves once the pointer moves past a threshold.
let downX = 0;
let downY = 0;
let downT = 0;
let downActive = false;
let dragging = false;

const CLICK_MOVE_TOLERANCE = 5; // px
const CLICK_TIME_TOLERANCE = 400; // ms

pillEl.addEventListener('mousedown', (e) => {
  if (e.button !== 0) return;
  downX = e.clientX;
  downY = e.clientY;
  downT = performance.now();
  downActive = true;
  dragging = false;
});

window.addEventListener('mousemove', (e) => {
  if (!downActive || dragging) return;
  const dx = Math.abs(e.clientX - downX);
  const dy = Math.abs(e.clientY - downY);
  if (dx > CLICK_MOVE_TOLERANCE || dy > CLICK_MOVE_TOLERANCE) {
    dragging = true;
    downActive = false;
    getCurrentWindow()
      .startDragging()
      .catch((err) => console.error('startDragging failed', err));
  }
});

window.addEventListener('mouseup', (e) => {
  if (!downActive || e.button !== 0) return;
  downActive = false;
  const dt = performance.now() - downT;
  if (!dragging && dt <= CLICK_TIME_TOLERANCE) {
    onDrink();
  }
});

async function onDrink() {
  playSplash();
  try {
    const tick = await invoke<Tick>('drink', { source: 'click' });
    currentTick = tick;
    render();
  } catch (err) {
    console.error('drink failed', err);
  }
}

async function onTogglePause() {
  try {
    const paused = currentTick?.mode !== 'paused';
    const tick = await invoke<Tick>('set_paused', { paused });
    currentTick = tick;
    render();
  } catch (err) {
    console.error('set_paused failed', err);
  }
}

function playSplash() {
  // Temporarily disable the linear transition and jump to full for a quick
  // "splash" refill, then let the next tick take over the normal drain.
  fillEl.classList.add('splash');
  fillEl.style.width = '100%';
  window.setTimeout(() => {
    fillEl.classList.remove('splash');
  }, 300);
}

// Right click and double click both open settings.
pillEl.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  invoke('open_settings').catch((err) => console.error('open_settings failed', err));
});

pillEl.addEventListener('dblclick', () => {
  invoke('open_settings').catch((err) => console.error('open_settings failed', err));
});

// --- Keyboard: Space = drink, P = pause toggle, while the widget has focus. ---
pillEl.addEventListener('keydown', (e) => {
  if (e.key === ' ' || e.key === 'Spacebar') {
    e.preventDefault();
    onDrink();
  } else if (e.key === 'p' || e.key === 'P') {
    e.preventDefault();
    onTogglePause();
  }
});

// --- Persist window position after a drag, debounced ---
let moveDebounce: number | undefined;

getCurrentWindow()
  .onMoved(({ payload }) => {
    if (moveDebounce !== undefined) window.clearTimeout(moveDebounce);
    moveDebounce = window.setTimeout(() => {
      invoke('save_position', { x: payload.x, y: payload.y }).catch((err) =>
        console.error('save_position failed', err)
      );
    }, 300);
  })
  .catch((err) => console.error('onMoved subscription failed', err));

// --- Compact layout: expand to full pill on hover, collapse 1.5s after leave. ---
let compactCollapseTimer: number | undefined;

pillEl.addEventListener('mouseenter', () => {
  if (compactCollapseTimer !== undefined) {
    window.clearTimeout(compactCollapseTimer);
    compactCollapseTimer = undefined;
  }
  pillEl.classList.add('expanded');
});

pillEl.addEventListener('mouseleave', () => {
  if (compactCollapseTimer !== undefined) window.clearTimeout(compactCollapseTimer);
  compactCollapseTimer = window.setTimeout(() => {
    pillEl.classList.remove('expanded');
  }, 1500);
});

init().catch((err) => console.error('widget init failed', err));
