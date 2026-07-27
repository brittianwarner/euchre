/**
 * `sound.svelte.ts` — every table sound cue, synthesised with WebAudio.
 *
 * No binary assets ship: each cue is a few oscillator/noise nodes with a gain
 * envelope, built fresh per call and torn down by the Web Audio graph itself
 * once its envelope reaches (near) zero — nothing here is loaded, decoded or
 * cached except one 1-second noise buffer shared by every noise-based cue.
 *
 * `docs/04-FRONTEND-UX.md` §11 names nine cues (`deal`, `place`, `lift`,
 * `trickTake`, `trumpCalled`, `euchre`, `march`, `gameWon`, `error`); this
 * module adds a tenth, `handWon`, because that table's two distinguished hand
 * outcomes (`euchre`, `march`) leave a plain `point`/`lone_point` — the
 * ordinary case of winning a hand without either extreme — with no cue of its
 * own, and this task's own brief names that outcome directly ("a warm chime on
 * taking a hand").
 *
 * Every cue is a no-op while `soundSettings.enabled` is `false` (muted by
 * default — see that class's doc comment) and while the tab is hidden, so nothing
 * plays into a background tab a user has already left.
 */

const STORAGE_KEY = 'euchre:sound-enabled';

/**
 * The one on/off switch every cue (and haptic) in this module checks before
 * doing anything. A class holding `$state`, not a store, per project
 * convention. Persisted to `localStorage` so a player's choice survives a
 * reload; defaults to `false` — sound is opt-in, never a surprise.
 */
export class SoundSettings {
	enabled = $state(false);

	constructor() {
		if (typeof window === 'undefined') return;
		try {
			this.enabled = window.localStorage.getItem(STORAGE_KEY) === '1';
		} catch {
			// Private browsing / storage disabled: fall back to the muted default.
		}
	}

	toggle(): void {
		this.set(!this.enabled);
	}

	set(value: boolean): void {
		this.enabled = value;
		if (typeof window === 'undefined') return;
		try {
			window.localStorage.setItem(STORAGE_KEY, value ? '1' : '0');
		} catch {
			// Same fallback as above — the toggle still works for this tab's session.
		}
	}
}

/** Shared instance — every component in `$lib/three` and `$lib/ui` reads this one. */
export const soundSettings = new SoundSettings();

/* -------------------------------------------------------------------------- */
/* WebAudio plumbing                                                          */
/* -------------------------------------------------------------------------- */

let ctx: AudioContext | null = null;

/** Lazily creates (and resumes) the one shared `AudioContext`. `null` outside the browser or if WebAudio is unsupported. */
function audioCtx(): AudioContext | null {
	if (typeof window === 'undefined') return null;
	const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext })
		.webkitAudioContext;
	if (!Ctor) return null;
	if (!ctx) ctx = new Ctor();
	if (ctx.state === 'suspended') void ctx.resume();
	return ctx;
}

if (typeof document !== 'undefined') {
	// "Resumed on the first user gesture" (doc §11): most browsers start an
	// `AudioContext` suspended until a genuine gesture. This warms it the moment
	// one occurs, so the *first* real cue (which may well be an AI's opening
	// bid tone) isn't the thing that has to carry that unlock.
	const unlock = () => {
		audioCtx();
		document.removeEventListener('pointerdown', unlock);
		document.removeEventListener('keydown', unlock);
	};
	document.addEventListener('pointerdown', unlock, { once: true });
	document.addEventListener('keydown', unlock, { once: true });
}

let noiseBuffer: AudioBuffer | null = null;

/** One second of white noise, regenerated only if the context's sample rate ever changes. */
function noiseBuffer_(c: AudioContext): AudioBuffer {
	if (noiseBuffer && noiseBuffer.sampleRate === c.sampleRate) return noiseBuffer;
	const buf = c.createBuffer(1, c.sampleRate, c.sampleRate);
	const data = buf.getChannelData(0);
	for (let i = 0; i < data.length; i++) data[i] = Math.random() * 2 - 1;
	noiseBuffer = buf;
	return buf;
}

interface ToneOpts {
	readonly type?: OscillatorType;
	readonly gain?: number;
	readonly attack?: number;
	readonly startOffset?: number;
	/** Second, quieter partial an octave above — cheap way to make a tone read as "struck" rather than "beeped". */
	readonly overtone?: boolean;
}

/** One enveloped oscillator: linear attack, exponential decay to (near) silence. */
function tone(c: AudioContext, freq: number, duration: number, opts: ToneOpts = {}): void {
	const { type = 'sine', gain = 0.2, attack = 0.008, startOffset = 0, overtone = false } = opts;
	const t0 = c.currentTime + startOffset;
	const master = c.createGain();
	master.gain.value = 0;
	master.connect(c.destination);
	master.gain.linearRampToValueAtTime(gain, t0 + attack);
	master.gain.exponentialRampToValueAtTime(0.0001, t0 + duration);

	const osc = c.createOscillator();
	osc.type = type;
	osc.frequency.value = freq;
	osc.connect(master);
	osc.start(t0);
	osc.stop(t0 + duration + 0.05);

	if (overtone) {
		const g2 = c.createGain();
		g2.gain.value = 0;
		g2.connect(c.destination);
		g2.gain.linearRampToValueAtTime(gain * 0.35, t0 + attack);
		g2.gain.exponentialRampToValueAtTime(0.0001, t0 + duration * 0.7);
		const osc2 = c.createOscillator();
		osc2.type = type;
		osc2.frequency.value = freq * 2;
		osc2.connect(g2);
		osc2.start(t0);
		osc2.stop(t0 + duration + 0.05);
	}
}

interface NoiseOpts {
	readonly duration: number;
	readonly gain?: number;
	readonly filterType?: BiquadFilterType;
	readonly filterFreq: number;
	/** If set, the filter's centre frequency glides there over `duration` — the "swept" cues. */
	readonly sweepTo?: number;
	readonly startOffset?: number;
	readonly q?: number;
}

/** Filtered white noise with a gain envelope — the felt/cloth/riffle family of cues. */
function noiseBurst(c: AudioContext, opts: NoiseOpts): void {
	const { duration, gain = 0.15, filterType = 'bandpass', filterFreq, sweepTo, startOffset = 0, q = 0.9 } = opts;
	const t0 = c.currentTime + startOffset;

	const src = c.createBufferSource();
	src.buffer = noiseBuffer_(c);

	const filter = c.createBiquadFilter();
	filter.type = filterType;
	filter.frequency.setValueAtTime(filterFreq, t0);
	filter.Q.value = q;
	if (sweepTo !== undefined) filter.frequency.exponentialRampToValueAtTime(Math.max(1, sweepTo), t0 + duration);

	const g = c.createGain();
	g.gain.value = 0;
	g.gain.linearRampToValueAtTime(gain, t0 + 0.012);
	g.gain.exponentialRampToValueAtTime(0.0001, t0 + duration);

	src.connect(filter).connect(g).connect(c.destination);
	src.start(t0);
	src.stop(t0 + duration + 0.02);
}

/** `-3..3` semitones, deterministic per string — used so five identical plays never sound identical (doc §11). */
function hashSemitone(id: string, spread = 3): number {
	let h = 2166136261;
	for (let i = 0; i < id.length; i++) {
		h ^= id.charCodeAt(i);
		h = Math.imul(h, 16777619);
	}
	const unit = ((h >>> 0) % 1000) / 1000; // 0..1, deterministic
	return (unit * 2 - 1) * spread;
}

function semitones(base: number, n: number): number {
	return base * Math.pow(2, n / 12);
}

/* -------------------------------------------------------------------------- */
/* Cues                                                                       */
/* -------------------------------------------------------------------------- */

function guard(): AudioContext | null {
	if (!soundSettings.enabled) return null;
	if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return null;
	return audioCtx();
}

/** Soft riffle. One per packet, not per card (doc §11) — callers decide how many times to fire this per deal. */
export function playDeal(): void {
	const c = guard();
	if (!c) return;
	noiseBurst(c, { duration: 0.18, gain: 0.1, filterType: 'bandpass', filterFreq: 2200, q: 0.6 });
}

/** Felt thud. `cardId` (e.g. `"JS"`) pitch-varies the thump so five plays never sound identical. */
export function playPlace(cardId?: string): void {
	const c = guard();
	if (!c) return;
	const shift = cardId ? hashSemitone(cardId) : 0;
	tone(c, semitones(150, shift), 0.12, { type: 'triangle', gain: 0.22, attack: 0.004 });
	noiseBurst(c, { duration: 0.07, gain: 0.05, filterType: 'lowpass', filterFreq: 900 });
}

/** Cloth brush — hover. Deliberately the quietest cue in the set. */
export function playLift(): void {
	const c = guard();
	if (!c) return;
	noiseBurst(c, { duration: 0.09, gain: 0.045, filterType: 'highpass', filterFreq: 3800, q: 0.5 });
}

/** Short swept collect — the trick sliding to the winner's pile. */
export function playTrickTake(): void {
	const c = guard();
	if (!c) return;
	noiseBurst(c, {
		duration: 0.26,
		gain: 0.13,
		filterType: 'bandpass',
		filterFreq: 3200,
		sweepTo: 500,
		q: 0.8
	});
}

const TRUMP_SUIT_FREQ: Readonly<Record<'S' | 'H' | 'D' | 'C', number>> = {
	S: 392.0, // G4
	H: 440.0, // A4
	D: 493.88, // B4
	C: 523.25 // C5
};

/** Single struck tone, one distinct pitch per suit. */
export function playTrumpCalled(suit: 'S' | 'H' | 'D' | 'C'): void {
	const c = guard();
	if (!c) return;
	tone(c, TRUMP_SUIT_FREQ[suit], 0.3, { type: 'triangle', gain: 0.24, attack: 0.006, overtone: true });
}

/** Descending minor third. */
export function playEuchre(): void {
	const c = guard();
	if (!c) return;
	tone(c, 440, 0.22, { type: 'sine', gain: 0.22, overtone: true });
	tone(c, semitones(440, -3), 0.24, { type: 'sine', gain: 0.22, startOffset: 0.14, overtone: true });
}

/** Ascending fanfare. */
export function playMarch(): void {
	const c = guard();
	if (!c) return;
	const notes = [349.23, 440, 523.25, 659.25]; // F4 A4 C5 E5
	notes.forEach((freq, i) => {
		tone(c, freq, 0.16, { type: 'triangle', gain: 0.2, startOffset: i * 0.08, overtone: true });
	});
}

/** Warm chime — the ordinary "you took the hand" beat (see the module doc for why this isn't in doc §11's list of nine). */
export function playHandWon(): void {
	const c = guard();
	if (!c) return;
	const chord = [523.25, 659.25, 783.99]; // C5 E5 G5
	chord.forEach((freq, i) => {
		tone(c, freq, 0.55, { type: 'sine', gain: 0.14, startOffset: i * 0.05, attack: 0.02 });
	});
}

/** Full cadence. */
export function playGameWon(): void {
	const c = guard();
	if (!c) return;
	const phrase: readonly [number, number][] = [
		[392.0, 0],
		[493.88, 0.12],
		[587.33, 0.24],
		[783.99, 0.4]
	];
	for (const [freq, offset] of phrase) {
		tone(c, freq, 0.5, { type: 'triangle', gain: 0.2, startOffset: offset, overtone: true });
	}
	// Final sustained chord.
	for (const freq of [392.0, 493.88, 587.33, 783.99]) {
		tone(c, freq, 0.7, { type: 'sine', gain: 0.12, startOffset: 0.55 });
	}
}

/** Dry, muted click — deliberately unmusical (an illegal tap). */
export function playError(): void {
	const c = guard();
	if (!c) return;
	noiseBurst(c, { duration: 0.08, gain: 0.16, filterType: 'lowpass', filterFreq: 550, q: 0.4 });
}

/* -------------------------------------------------------------------------- */
/* Haptics                                                                     */
/* -------------------------------------------------------------------------- */

/**
 * `navigator.vibrate` wrapper — a silent no-op on iOS Safari (which has no
 * such API) and anywhere `soundSettings.enabled` is `false`, matching doc
 * §11's "both channels honour the Settings toggles."
 */
export function vibrate(pattern: number | readonly number[]): void {
	if (!soundSettings.enabled) return;
	if (typeof navigator === 'undefined' || typeof navigator.vibrate !== 'function') return;
	navigator.vibrate(pattern as number | number[]);
}
