import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { Settings } from './types';

const intervalRange = document.getElementById('interval-range') as HTMLInputElement;
const intervalNumber = document.getElementById('interval-number') as HTMLInputElement;
const opacityRange = document.getElementById('opacity-range') as HTMLInputElement;
const opacityValue = document.getElementById('opacity-value') as HTMLSpanElement;
const showTextCheckbox = document.getElementById('show-text') as HTMLInputElement;
const showCountCheckbox = document.getElementById('show-count') as HTMLInputElement;
const quitBtn = document.getElementById('quit-btn') as HTMLButtonElement;

let current: Settings = {
  intervalMin: 45,
  opacity: 0.9,
  showText: true,
  showCount: true,
};

let applying = false; // guard against feedback loop while we render incoming state

function renderFromSettings(s: Settings) {
  applying = true;
  intervalRange.value = String(s.intervalMin);
  intervalNumber.value = String(s.intervalMin);
  opacityRange.value = String(Math.round(s.opacity * 100));
  opacityValue.textContent = `${Math.round(s.opacity * 100)}%`;
  showTextCheckbox.checked = s.showText;
  showCountCheckbox.checked = s.showCount;
  applying = false;
}

function readForm(): Settings {
  return {
    intervalMin: clamp(Number(intervalNumber.value) || current.intervalMin, 10, 180),
    opacity: clamp(Number(opacityRange.value) / 100 || current.opacity, 0.3, 1.0),
    showText: showTextCheckbox.checked,
    showCount: showCountCheckbox.checked,
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

// Keep the range and number inputs for interval in sync as the user types.
intervalRange.addEventListener('input', () => {
  intervalNumber.value = intervalRange.value;
  scheduleApply();
});
intervalNumber.addEventListener('input', () => {
  intervalRange.value = intervalNumber.value;
  scheduleApply();
});

opacityRange.addEventListener('input', () => {
  opacityValue.textContent = `${opacityRange.value}%`;
  scheduleApply();
});

showTextCheckbox.addEventListener('input', scheduleApply);
showCountCheckbox.addEventListener('input', scheduleApply);

(document.getElementById('settings-form') as HTMLFormElement).addEventListener('submit', (e) =>
  e.preventDefault()
);

// Escape hides the settings window (same as the close button).
window.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    getCurrentWindow().hide().catch((err) => console.error('hide failed', err));
  }
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
