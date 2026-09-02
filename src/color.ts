// Pure color + formatting helpers for the Tide widget.
// No dependencies; OKLCH math implemented manually (Björn Ottosson's OKLab).

import type { Tick } from './types';

interface RGB {
  r: number; // 0..1 linear-agnostic (sRGB gamma-encoded, 0..1)
  g: number;
  b: number;
}

interface OKLCH {
  l: number; // 0..1
  c: number; // chroma, >= 0
  h: number; // degrees 0..360
}

function hexToRgb(hex: string): RGB {
  const m = hex.replace('#', '');
  const r = parseInt(m.substring(0, 2), 16) / 255;
  const g = parseInt(m.substring(2, 4), 16) / 255;
  const b = parseInt(m.substring(4, 6), 16) / 255;
  return { r, g, b };
}

function srgbToLinear(v: number): number {
  return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}

function linearToSrgb(v: number): number {
  const c = Math.max(0, Math.min(1, v));
  return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
}

function rgbToOklch(rgb: RGB): OKLCH {
  const lr = srgbToLinear(rgb.r);
  const lg = srgbToLinear(rgb.g);
  const lb = srgbToLinear(rgb.b);

  const l = 0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb;
  const m = 0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb;
  const s = 0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb;

  const l_ = Math.cbrt(l);
  const m_ = Math.cbrt(m);
  const s_ = Math.cbrt(s);

  const L = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
  const a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
  const b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;

  const c = Math.sqrt(a * a + b * b);
  let h = (Math.atan2(b, a) * 180) / Math.PI;
  if (h < 0) h += 360;

  return { l: L, c, h };
}

function oklchToRgb(lch: OKLCH): RGB {
  const hRad = (lch.h * Math.PI) / 180;
  const a = lch.c * Math.cos(hRad);
  const b = lch.c * Math.sin(hRad);
  const L = lch.l;

  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.2914855480 * b;

  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;

  const lr = +4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
  const lg = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
  const lb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;

  return {
    r: linearToSrgb(lr),
    g: linearToSrgb(lg),
    b: linearToSrgb(lb),
  };
}

function rgbToHex(rgb: RGB): string {
  const to255 = (v: number) => Math.round(Math.max(0, Math.min(1, v)) * 255);
  const r = to255(rgb.r).toString(16).padStart(2, '0');
  const g = to255(rgb.g).toString(16).padStart(2, '0');
  const b = to255(rgb.b).toString(16).padStart(2, '0');
  return `#${r}${g}${b}`;
}

// Stops per CONTRACT.md, sorted descending by fill.
const STOPS: Array<{ fill: number; hex: string }> = [
  { fill: 1.0, hex: '#3B82F6' },
  { fill: 0.6, hex: '#22C55E' },
  { fill: 0.45, hex: '#EAB308' },
  { fill: 0.3, hex: '#F97316' },
  { fill: 0.0, hex: '#EF4444' },
];

const STOP_LCH = STOPS.map((s) => ({ fill: s.fill, lch: rgbToOklch(hexToRgb(s.hex)) }));

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

// Interpolate hue along the shorter arc.
function lerpHue(a: number, b: number, t: number): number {
  let diff = b - a;
  diff = ((diff + 180) % 360 + 360) % 360 - 180;
  let h = a + diff * t;
  h = ((h % 360) + 360) % 360;
  return h;
}

/** Returns a CSS hex color for a given fill (0..1), interpolated in OKLCH between the CONTRACT.md stops. */
export function fillColor(fill: number): string {
  const f = Math.max(0, Math.min(1, fill));

  // Find bracketing stops (STOP_LCH is sorted descending by fill).
  for (let i = 0; i < STOP_LCH.length - 1; i++) {
    const upper = STOP_LCH[i];
    const lower = STOP_LCH[i + 1];
    if (f <= upper.fill && f >= lower.fill) {
      const span = upper.fill - lower.fill;
      const t = span === 0 ? 0 : (upper.fill - f) / span; // 0 at upper, 1 at lower
      const l = lerp(upper.lch.l, lower.lch.l, t);
      const c = lerp(upper.lch.c, lower.lch.c, t);
      const h = lerpHue(upper.lch.h, lower.lch.h, t);
      return rgbToHex(oklchToRgb({ l, c, h }));
    }
  }
  // Fallback (shouldn't happen given stops span 0..1).
  return STOPS[STOPS.length - 1].hex;
}

// WCAG relative luminance from sRGB hex.
function relativeLuminance(hex: string): number {
  const rgb = hexToRgb(hex);
  const lin = (v: number) => srgbToLinear(v);
  const r = lin(rgb.r);
  const g = lin(rgb.g);
  const b = lin(rgb.b);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrastRatio(l1: number, l2: number): number {
  const lighter = Math.max(l1, l2);
  const darker = Math.min(l1, l2);
  return (lighter + 0.05) / (darker + 0.05);
}

/** Picks white or near-black text for >= 4.5:1 contrast against bg (hex color). */
export function textColorOn(bg: string): string {
  const bgLum = relativeLuminance(bg);
  const white = '#FFFFFF';
  const black = '#111111';
  const whiteContrast = contrastRatio(bgLum, relativeLuminance(white));
  const blackContrast = contrastRatio(bgLum, relativeLuminance(black));
  // Prefer whichever meets 4.5:1; if both/neither, pick the higher contrast.
  if (whiteContrast >= 4.5 && whiteContrast >= blackContrast) return white;
  if (blackContrast >= 4.5) return black;
  return whiteContrast >= blackContrast ? white : black;
}

/** Formats remaining/overdue time for display: "42 min" / "8 min" / "+12 min" / "<1 min". */
export function formatRemaining(tick: Tick): string {
  if (tick.zone === 'overdue') {
    const overdueMin = Math.floor(tick.overdueMs / 60000);
    if (overdueMin < 1) return '<1 min';
    return `+${overdueMin} min`;
  }
  const remainingMin = Math.floor(tick.remainingMs / 60000);
  if (tick.remainingMs > 0 && remainingMin < 1) return '<1 min';
  return `${remainingMin} min`;
}
