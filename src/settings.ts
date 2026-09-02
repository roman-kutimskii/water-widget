import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { playDropSound } from './sound';
import type { Settings } from './types';

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
};

let current: Settings = { ...DEFAULTS };

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

async function init() {
  try {
    const settings = await invoke<Settings>('get_settings');
    current = settings;
    renderFromSettings(current);
  } catch (err) {
    console.error('get_settings failed', err);
  }

  await listen<Settings>('settings-changed', (event) => {
    // Reflect external changes (e.g. from another window) without
    // clobbering an in-flight edit.
    if (applying) return;
    current = event.payload;
    renderFromSettings(current);
  });
}

init().catch((err) => console.error('settings init failed', err));
