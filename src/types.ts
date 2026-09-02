// Types mirroring CONTRACT.md exactly (MVP + v0.2 additions).

export interface Tick {
  fill: number; // 0..1, 1 = just drank
  zone: 'fresh' | 'fading' | 'urgent' | 'overdue';
  remainingMs: number; // >= 0; 0 when overdue
  overdueMs: number; // >= 0; 0 when not overdue
  todayCount: number;
  intervalMs: number;
  lastDrinkTs: number; // unix ms
  // v0.2
  mode: 'active' | 'paused' | 'sleeping';
  quiet: boolean;
  snoozeMs: number;
  pausedSince: number | null;
  // v0.3
  streak: number;
}

export interface Nudge {
  kind: 'overdue' | 'repeat' | 'welcome-back' | 'auto-resume';
  overdueMs: number;
}

export interface Settings {
  // MVP
  intervalMin: number; // 10..180, default 45
  opacity: number; // 0.3..1.0, default 0.9
  showText: boolean; // default true
  showCount: boolean; // default true
  // Timing
  activeStart: string; // "HH:MM", default "08:00"
  activeEnd: string; // default "22:00"
  quietStart: string; // default "22:00"
  quietEnd: string; // default "08:00"
  dailyGoal: number; // 1..30, default 8
  // Behavior
  alwaysOnTop: boolean; // default true
  clickThrough: boolean; // default false
  autostart: boolean; // default false
  hotkeyEnabled: boolean; // default true
  hotkey: string; // default "Ctrl+Alt+W"
  // Alerts
  toastEnabled: boolean; // default true
  nudgeEveryMin: number; // 1..60, default 10
  nudgeMax: number; // 0..10, default 3
  soundEnabled: boolean; // default false
  soundVolume: number; // 0..1, default 0.5
  // v0.3 Look
  layout: 'horizontal' | 'vertical' | 'compact'; // default 'horizontal'
  scale: number; // 0.75..1.5, default 1.0
  colorPreset: 'default' | 'colorblind' | 'mono'; // default 'default'
  reducedMotion: 'system' | 'on' | 'off'; // default 'system'
}

export interface DayStat {
  dayKey: string; // "YYYY-MM-DD"
  drinks: number;
  avgGapMin: number | null;
  longestOverdueMin: number;
  goalMet: boolean;
}

export interface Stats {
  days: DayStat[]; // last 14 days, oldest first
  streak: number;
  bestStreak: number;
  totalDrinks: number;
}
