import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { fillColor, formatRemaining } from './color';
import type { Tick, Settings } from './types';

const pillEl = document.getElementById('pill') as HTMLDivElement;
const fillEl = document.getElementById('fill') as HTMLDivElement;
const textEl = document.getElementById('text') as HTMLDivElement;

let currentTick: Tick | null = null;
let currentSettings: Settings | null = null;

function render() {
  if (!currentTick) return;
  const tick = currentTick;
  const color = fillColor(tick.fill);

  fillEl.style.width = `${tick.fill * 100}%`;
  fillEl.style.backgroundColor = color;

  pillEl.classList.toggle('overdue', tick.zone === 'overdue');

  if (currentSettings) {
    pillEl.style.opacity = String(currentSettings.opacity);
  }

  const showText = currentSettings ? currentSettings.showText : true;
  const showCount = currentSettings ? currentSettings.showCount : true;

  if (showText) {
    let label = formatRemaining(tick);
    if (showCount) {
      label += ` · ${tick.todayCount}`;
    }
    textEl.textContent = label;
    textEl.classList.remove('hidden');
  } else {
    textEl.classList.add('hidden');
  }

  pillEl.title = tooltipFor(tick);
}

function tooltipFor(tick: Tick): string {
  const last = new Date(tick.lastDrinkTs);
  const next = new Date(tick.lastDrinkTs + tick.intervalMs);
  const fmt = (d: Date) =>
    `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  return `Last drink ${fmt(last)} · Next ${fmt(next)} · Today ${tick.todayCount}`;
}

async function init() {
  const [tick, settings] = await Promise.all([
    invoke<Tick>('get_tick'),
    invoke<Settings>('get_settings'),
  ]);
  currentTick = tick;
  currentSettings = settings;
  render();

  await listen<Tick>('tick', (event) => {
    currentTick = event.payload;
    render();
  });

  await listen<Settings>('settings-changed', (event) => {
    currentSettings = event.payload;
    render();
  });
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

pillEl.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  downX = e.clientX;
  downY = e.clientY;
  downT = performance.now();
  downActive = true;
  dragging = false;
});

window.addEventListener("mousemove", (e) => {
  if (!downActive || dragging) return;
  const dx = Math.abs(e.clientX - downX);
  const dy = Math.abs(e.clientY - downY);
  if (dx > CLICK_MOVE_TOLERANCE || dy > CLICK_MOVE_TOLERANCE) {
    dragging = true;
    downActive = false;
    getCurrentWindow()
      .startDragging()
      .catch((err) => console.error("startDragging failed", err));
  }
});

window.addEventListener("mouseup", (e) => {
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

init().catch((err) => console.error('widget init failed', err));
