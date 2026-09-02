import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { playDropSound } from './sound';
import { renderStatsSvg } from './chart';
import type { Settings, Stats, Tick } from './types';

// --- Timing ---
const intervalRange = document.getElementById('interval-range') as HTMLInputElement;
const intervalNumber = document.getElementById('interval-number') as HTMLInputElement;
const activeStart = document.getElementById('active-start') as HTMLInputElement;
const activeEnd = document.getElementById('active-end') as HTMLInputElement;
const quietStart = document.getElementById('quiet-start') as HTMLInputElement;
const quietEnd = document.getElementById('quiet-end') as HTMLInputElement;
const dailyGoal = document.getElementById('daily-goal') as HTMLInputElement;

// --- Look ---
const opacityRange = document.getElementById('opacity-range') as HTMLInputElement;
const opacityValue = document.getElementById('opacity-value') as HTMLSpanElement;
const showTextCheckbox = document.getElementById('show-text') as HTMLInputElement;
const showCountCheckbox = document.getElementById('show-count') as HTMLInputElement;
const layoutSelect = document.getElementById('layout-select') as HTMLSelectElement;
const scaleRange = document.getElementById('scale-range') as HTMLInputElement;
const scaleValue = document.getElementById('scale-value') as HTMLSpanElement;
const colorPresetSelect = document.getElementById('color-preset-select') as HTMLSelectElement;
const reducedMotionSelect = document.getElementById('reduced-motion-select') as HTMLSelectElement;

// --- Behavior ---
const alwaysOnTop = document.getElementById('always-on-top') as HTMLInputElement;
const clickThrough = document.getElementById('click-through') as HTMLInputElement;
const autostart = document.getElementById('autostart') as HTMLInputElement;
const hotkeyEnabled = document.getElementById('hotkey-enabled') as HTMLInputElement;
const hotkeyText = document.getElementById('hotkey-text') as HTMLInputElement;

// --- Alerts ---
const toastEnabled = document.getElementById('toast-enabled') as HTMLInputElement;
const nudgeEvery = document.getElementById('nudge-every') as HTMLInputElement;
const nudgeMax = document.getElementById('nudge-max') as HTMLInputElement;
const soundEnabled = document.getElementById('sound-enabled') as HTMLInputElement;
const soundVolume = document.getElementById('sound-volume') as HTMLInputElement;
const soundVolumeValue = document.getElementById('sound-volume-value') as HTMLSpanElement;
const testSoundBtn = document.getElementById('test-sound-btn') as HTMLButtonElement;

// --- Stats ---
const statsSummary = document.getElementById('stats-summary') as HTMLDivElement;
const statsChart = document.getElementById('stats-chart') as HTMLDivElement;

// --- Data ---
const openDataDirBtn = document.getElementById('open-data-dir-btn') as HTMLButtonElement;
const resetAllBtn = document.getElementById('reset-all-btn') as HTMLButtonElement;
const dataStatus = document.getElementById('data-status') as HTMLDivElement;

// --- Footer ---
const resetTodayBtn = document.getElementById('reset-today-btn') as HTMLButtonElement;
const quitBtn = document.getElementById('quit-btn') as HTMLButtonElement;

const DEFAULTS: Settings = {
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
  layout: 'horizontal',
  scale: 1.0,
  colorPreset: 'default',
  reducedMotion: 'system',
};

let current: Settings = { ...DEFAULTS };
let lastTodayCount: number | null = null;

let applying = false; // guard against feedback loop while we render incoming state

function renderFromSettings(s: Settings) {
  applying = true;
  intervalRange.value = String(s.intervalMin);
  intervalNumber.value = String(s.intervalMin);
  activeStart.value = s.activeStart;
  activeEnd.value = s.activeEnd;
  quietStart.value = s.quietStart;
  quietEnd.value = s.quietEnd;
  dailyGoal.value = String(s.dailyGoal);

  opacityRange.value = String(Math.round(s.opacity * 100));
  opacityValue.textContent = `${Math.round(s.opacity * 100)}%`;
  showTextCheckbox.checked = s.showText;
  showCountCheckbox.checked = s.showCount;

  alwaysOnTop.checked = s.alwaysOnTop;
  clickThrough.checked = s.clickThrough;
  autostart.checked = s.autostart;
  hotkeyEnabled.checked = s.hotkeyEnabled;
  hotkeyText.value = s.hotkey;

  toastEnabled.checked = s.toastEnabled;
  nudgeEvery.value = String(s.nudgeEveryMin);
  nudgeMax.value = String(s.nudgeMax);
  soundEnabled.checked = s.soundEnabled;
  soundVolume.value = String(Math.round(s.soundVolume * 100));
  soundVolumeValue.textContent = `${Math.round(s.soundVolume * 100)}%`;

  layoutSelect.value = s.layout;
  scaleRange.value = String(Math.round(s.scale * 100));
  scaleValue.textContent = `${Math.round(s.scale * 100)}%`;
  colorPresetSelect.value = s.colorPreset;
  reducedMotionSelect.value = s.reducedMotion;
  applying = false;
}

function readForm(): Settings {
  return {
    intervalMin: clamp(Number(intervalNumber.value) || current.intervalMin, 10, 180),
    opacity: clamp(Number(opacityRange.value) / 100 || current.opacity, 0.3, 1.0),
    showText: showTextCheckbox.checked,
    showCount: showCountCheckbox.checked,
    activeStart: activeStart.value || current.activeStart,
    activeEnd: activeEnd.value || current.activeEnd,
    quietStart: quietStart.value || current.quietStart,
    quietEnd: quietEnd.value || current.quietEnd,
    dailyGoal: clamp(Number(dailyGoal.value) || current.dailyGoal, 1, 30),
    alwaysOnTop: alwaysOnTop.checked,
    clickThrough: clickThrough.checked,
    autostart: autostart.checked,
    hotkeyEnabled: hotkeyEnabled.checked,
    hotkey: hotkeyText.value.trim() || current.hotkey,
    toastEnabled: toastEnabled.checked,
    nudgeEveryMin: clamp(Number(nudgeEvery.value) || current.nudgeEveryMin, 1, 60),
    nudgeMax: clamp(Number(nudgeMax.value) || 0, 0, 10),
    soundEnabled: soundEnabled.checked,
    soundVolume: clamp(
      Number.isFinite(Number(soundVolume.value)) ? Number(soundVolume.value) / 100 : current.soundVolume,
      0,
      1
    ),
    layout: (layoutSelect.value as Settings['layout']) || current.layout,
    scale: clamp((Number(scaleRange.value) || current.scale * 100) / 100, 0.75, 1.5),
    colorPreset: (colorPresetSelect.value as Settings['colorPreset']) || current.colorPreset,
    reducedMotion: (reducedMotionSelect.value as Settings['reducedMotion']) || current.reducedMotion,
  };
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v));
}

let debounceHandle: number | undefined;

function scheduleApply() {
  if (applying) return;
  if (debounceHandle !== undefined) window.clearTimeout(debounceHandle);
  debounceHandle = window.setTimeout(applyChanges, 150);
}

async function applyChanges() {
  current = readForm();
  try {
    const confirmed = await invoke<Settings>('set_settings', { settings: current });
    current = confirmed;
    renderFromSettings(current);
  } catch (err) {
    console.error('set_settings failed', err);
  }
}

// Apply immediately (bypassing debounce) — used for the hotkey field, which
// should only apply on blur/Enter, not every keystroke.
async function applyChangesNow() {
  if (debounceHandle !== undefined) {
    window.clearTimeout(debounceHandle);
    debounceHandle = undefined;
  }
  await applyChanges();
}

// --- Timing listeners ---
intervalRange.addEventListener('input', () => {
  intervalNumber.value = intervalRange.value;
  scheduleApply();
});
intervalNumber.addEventListener('input', () => {
  intervalRange.value = intervalNumber.value;
  scheduleApply();
});
activeStart.addEventListener('input', scheduleApply);
activeEnd.addEventListener('input', scheduleApply);
quietStart.addEventListener('input', scheduleApply);
quietEnd.addEventListener('input', scheduleApply);
dailyGoal.addEventListener('input', scheduleApply);

// --- Look listeners ---
opacityRange.addEventListener('input', () => {
  opacityValue.textContent = `${opacityRange.value}%`;
  scheduleApply();
});
showTextCheckbox.addEventListener('input', scheduleApply);
showCountCheckbox.addEventListener('input', scheduleApply);
layoutSelect.addEventListener('change', scheduleApply);
scaleRange.addEventListener('input', () => {
  scaleValue.textContent = `${scaleRange.value}%`;
  scheduleApply();
});
colorPresetSelect.addEventListener('change', scheduleApply);
reducedMotionSelect.addEventListener('change', scheduleApply);

// --- Behavior listeners ---
alwaysOnTop.addEventListener('input', scheduleApply);
clickThrough.addEventListener('input', scheduleApply);
autostart.addEventListener('input', scheduleApply);
hotkeyEnabled.addEventListener('input', scheduleApply);
// Hotkey text applies only on change (blur/Enter), never per keystroke.
hotkeyText.addEventListener('change', () => {
  void applyChangesNow();
});
hotkeyText.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    hotkeyText.blur();
  }
});

// --- Alerts listeners ---
toastEnabled.addEventListener('input', scheduleApply);
nudgeEvery.addEventListener('input', scheduleApply);
nudgeMax.addEventListener('input', scheduleApply);
soundEnabled.addEventListener('input', scheduleApply);
soundVolume.addEventListener('input', () => {
  soundVolumeValue.textContent = `${soundVolume.value}%`;
  scheduleApply();
});

testSoundBtn.addEventListener('click', () => {
  playDropSound(Number(soundVolume.value) / 100);
});

(document.getElementById('settings-form') as HTMLFormElement).addEventListener('submit', (e) =>
  e.preventDefault()
);

// The window is created once at startup; closing it only hides it so the next
// open is instant. Intercepting here works reliably on Windows, unlike hide()
// from the Rust close handler.
getCurrentWindow()
  .onCloseRequested(async (event) => {
    event.preventDefault();
    try {
      await getCurrentWindow().hide();
    } catch (err) {
      console.error('hide failed', err);
    }
  })
  .catch((err) => console.error('onCloseRequested failed', err));

// Escape hides the settings window (same as the close button).
window.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    getCurrentWindow().hide().catch((err) => console.error('hide failed', err));
  }
});

resetTodayBtn.addEventListener('click', () => {
  invoke('reset_today').catch((err) => console.error('reset_today failed', err));
});

quitBtn.addEventListener('click', () => {
  invoke('quit').catch((err) => console.error('quit failed', err));
});

// --- Stats ---
async function loadStats() {
  try {
    const stats = await invoke<Stats>('get_stats');
    statsSummary.textContent = `Streak ${stats.streak} days · Best ${stats.bestStreak} · Total ${stats.totalDrinks} drinks`;
    statsChart.innerHTML = renderStatsSvg(stats, current.dailyGoal);
  } catch (err) {
    console.error('get_stats failed', err);
  }
}

// --- Data ---
openDataDirBtn.addEventListener('click', () => {
  invoke('open_data_dir').catch((err) => console.error('open_data_dir failed', err));
});

// Two-step confirmation inside the page (the browser confirm() shows an ugly
// "localhost says" dialog). First click arms the button for 4 s.
let resetArmTimer: number | undefined;
function disarmResetAll() {
  resetAllBtn.textContent = 'Reset all';
  resetAllBtn.classList.remove('armed');
  if (resetArmTimer !== undefined) {
    window.clearTimeout(resetArmTimer);
    resetArmTimer = undefined;
  }
}

resetAllBtn.addEventListener('click', async () => {
  if (!resetAllBtn.classList.contains('armed')) {
    resetAllBtn.classList.add('armed');
    resetAllBtn.textContent = 'Click again to delete all history';
    resetArmTimer = window.setTimeout(disarmResetAll, 4000);
    return;
  }
  disarmResetAll();
  try {
    await invoke<Tick>('reset_all');
    dataStatus.textContent = 'All history and streaks have been reset.';
    await loadStats();
  } catch (err) {
    console.error('reset_all failed', err);
    dataStatus.textContent = 'Reset failed.';
  }
});

async function init() {
  try {
    const settings = await invoke<Settings>('get_settings');
    current = settings;
    renderFromSettings(current);
  } catch (err) {
    console.error('get_settings failed', err);
  }

  await loadStats();

  try {
    const tick = await invoke<Tick>('get_tick');
    lastTodayCount = tick.todayCount;
  } catch (err) {
    console.error('get_tick failed', err);
  }

  await listen<Settings>('settings-changed', (event) => {
    // Reflect external changes (e.g. from another window) without
    // clobbering an in-flight edit.
    if (applying) return;
    current = event.payload;
    renderFromSettings(current);
  });

  await listen<Tick>('tick', (event) => {
    const tick = event.payload;
    if (lastTodayCount === null || tick.todayCount !== lastTodayCount) {
      lastTodayCount = tick.todayCount;
      void loadStats();
    }
  });
}

init().catch((err) => console.error('settings init failed', err));
