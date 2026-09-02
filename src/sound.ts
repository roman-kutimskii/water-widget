// Synthesized water-drop sound, shared by widget and settings.
// No asset file: a short sine sweep + a tiny noise burst, WebAudio only.

let sharedCtx: AudioContext | null = null;

function getCtx(): AudioContext | null {
  try {
    if (!sharedCtx) {
      const Ctor =
        window.AudioContext ||
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctor) return null;
      sharedCtx = new Ctor();
    }
    if (sharedCtx.state === 'suspended') {
      sharedCtx.resume().catch(() => {});
    }
    return sharedCtx;
  } catch {
    return null;
  }
}

/** Plays a ~120ms synthesized water-drop at the given volume (0..1). Safe to call anywhere; never throws. */
export function playDropSound(volume: number): void {
  try {
    const ctx = getCtx();
    if (!ctx) return;
    const vol = Math.max(0, Math.min(1, volume));
    if (vol <= 0) return;
    const now = ctx.currentTime;
    const duration = 0.12;

    // Sine sweep 900 -> 300 Hz with exponential gain decay.
    const osc = ctx.createOscillator();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(900, now);
    osc.frequency.exponentialRampToValueAtTime(300, now + duration);

    const gain = ctx.createGain();
    gain.gain.setValueAtTime(vol * 0.5, now);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + duration);

    osc.connect(gain).connect(ctx.destination);
    osc.start(now);
    osc.stop(now + duration + 0.02);

    // Tiny noise burst for the "drop" transient.
    const noiseDuration = 0.02;
    const bufferSize = Math.max(1, Math.floor(ctx.sampleRate * noiseDuration));
    const buffer = ctx.createBuffer(1, bufferSize, ctx.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < bufferSize; i++) {
      data[i] = (Math.random() * 2 - 1) * (1 - i / bufferSize);
    }
    const noise = ctx.createBufferSource();
    noise.buffer = buffer;
    const noiseGain = ctx.createGain();
    noiseGain.gain.setValueAtTime(vol * 0.35, now);
    noiseGain.gain.exponentialRampToValueAtTime(0.0001, now + noiseDuration);
    noise.connect(noiseGain).connect(ctx.destination);
    noise.start(now);

    osc.onended = () => {
      try {
        osc.disconnect();
        gain.disconnect();
      } catch {
        /* ignore */
      }
    };
    noise.onended = () => {
      try {
        noise.disconnect();
        noiseGain.disconnect();
      } catch {
        /* ignore */
      }
    };
  } catch (err) {
    console.error('playDropSound failed', err);
  }
}
