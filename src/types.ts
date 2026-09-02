// Types mirroring CONTRACT.md exactly. Frozen for MVP.

export interface Tick {
  fill: number; // 0..1, 1 = just drank
  zone: 'fresh' | 'fading' | 'urgent' | 'overdue';
  remainingMs: number; // >= 0; 0 when overdue
  overdueMs: number; // >= 0; 0 when not overdue
  todayCount: number;
  intervalMs: number;
  lastDrinkTs: number; // unix ms
}

export interface Settings {
  intervalMin: number; // 10..180, default 45
  opacity: number; // 0.3..1.0, default 0.9
  showText: boolean; // default true
  showCount: boolean; // default true
}
