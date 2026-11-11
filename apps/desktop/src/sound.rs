//! Sound cues (06 §15). The table's cues are synthesised, as the web app's are: no asset files,
//! just enveloped oscillators and filtered noise. D14: sound is off until the player turns it on.
//!
//! How it fits together:
//! - app.rs `poll` drains `Presentation.hooks.sound` every frame and calls [`Sound::play`] for
//!   each cue, but only while `prefs.sound` is on, the window is visible and the run is not
//!   headless. The queue drains either way, so turning sound on never replays a backlog.
//! - `AppState::toggle_sound` flips `prefs.sound` and calls [`Sound::set_enabled`].
//! - [`recipe`] is the 06 §15.2 table as data and [`render`] turns one cue into a mono buffer.
//!   Both are pure, so the tests check every cue without an audio device.
//! - [`Sound::play`] opens the output lazily, on the first cue it is asked to play while on,
//!   on a thread of its own that holds the device and caches each rendered cue. It never opens
//!   one in a headless run or outside the `euchre` app binary (the test binaries and `--snap`
//!   runs have no business with a speaker). When no device opens it logs one line and stays
//!   silent for the rest of the run.

#[cfg(not(target_arch = "wasm32"))]
use std::num::NonZero;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, Sender};

use euchre_protocol::{Card, Suit};

use crate::screens::table::present::Cue;

/// The rate every cue is rendered at; the mixer resamples to the device.
pub const SAMPLE_RATE: u32 = 44_100;

/// Where an exponential decay ends (06 §15.2: "exponential decay to 0.0001").
const FLOOR: f32 = 0.0001;

/// The noise bursts' fixed linear attack (the web `noiseBurst`).
const NOISE_ATTACK: f32 = 0.012;

/// An oscillator shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    /// A sine.
    Sine,
    /// A triangle.
    Triangle,
}

/// A noise filter (WebAudio `BiquadFilterNode` types).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// Band-pass, `q` linear.
    BandPass,
    /// Low-pass, `q` in dB (as WebAudio reads it).
    LowPass,
    /// High-pass, `q` in dB.
    HighPass,
}

/// One voice of a cue: what 06 §15.2 lists, one row of the recipe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Voice {
    /// An enveloped oscillator: a linear ramp to `gain` over `attack`, then an exponential
    /// decay to 0.0001 at `duration`. `overtone` adds the octave at 35 % gain, gone by 70 % of
    /// the duration.
    Tone {
        /// The shape.
        wave: Wave,
        /// Hz.
        freq: f32,
        /// Seconds from the voice's start to the end of its decay.
        duration: f32,
        /// Peak gain, 0..1.
        gain: f32,
        /// Seconds of linear attack.
        attack: f32,
        /// Seconds after the cue starts.
        start: f32,
        /// The octave partial.
        overtone: bool,
    },
    /// Filtered white noise with a 12 ms linear attack and an exponential decay.
    Noise {
        /// Seconds.
        duration: f32,
        /// Peak gain, 0..1.
        gain: f32,
        /// The filter.
        filter: Filter,
        /// The filter's frequency, Hz.
        freq: f32,
        /// When set, the frequency glides here exponentially over the duration.
        sweep_to: Option<f32>,
        /// The filter's Q.
        q: f32,
        /// Seconds after the cue starts.
        start: f32,
    },
}

/// The trump-called pitch of each suit (06 §15.2: S G4, H A4, D B4, C C5).
pub fn trump_freq(suit: Suit) -> f32 {
    match suit {
        Suit::Spades => 392.0,
        Suit::Hearts => 440.0,
        Suit::Diamonds => 493.88,
        Suit::Clubs => 523.25,
    }
}

/// `hashSemitone`: the deterministic −3..+3 semitone shift of a card's place thud (FNV-1a over
/// the card id, `((h % 1000) / 1000 * 2 - 1) * 3`), so four cards in a trick sound like four
/// cards.
pub fn place_shift(card: Card) -> f32 {
    let mut h: u32 = 2_166_136_261;
    for unit in card.to_string().encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(16_777_619);
    }
    let unit = (h % 1000) as f32 / 1000.0;
    (unit * 2.0 - 1.0) * 3.0
}

fn semitones(base: f32, n: f32) -> f32 {
    base * 2f32.powf(n / 12.0)
}

fn tone(wave: Wave, freq: f32, duration: f32, gain: f32) -> Voice {
    Voice::Tone {
        wave,
        freq,
        duration,
        gain,
        attack: 0.008,
        start: 0.0,
        overtone: false,
    }
}

fn noise(duration: f32, gain: f32, filter: Filter, freq: f32, q: f32) -> Voice {
    Voice::Noise {
        duration,
        gain,
        filter,
        freq,
        sweep_to: None,
        q,
        start: 0.0,
    }
}

fn with(voice: Voice, f: impl FnOnce(&mut f32, &mut f32, &mut bool)) -> Voice {
    match voice {
        Voice::Tone {
            wave,
            freq,
            duration,
            gain,
            mut attack,
            mut start,
            mut overtone,
        } => {
            f(&mut attack, &mut start, &mut overtone);
            Voice::Tone {
                wave,
                freq,
                duration,
                gain,
                attack,
                start,
                overtone,
            }
        }
        other => other,
    }
}

/// The voices of `cue`, verbatim from 06 §15.2.
pub fn recipe(cue: &Cue) -> Vec<Voice> {
    let at = |v: Voice, start: f32| with(v, |_, s, _| *s = start);
    let over = |v: Voice| with(v, |_, _, o| *o = true);
    match *cue {
        Cue::Deal => vec![noise(0.18, 0.1, Filter::BandPass, 2200.0, 0.6)],
        Cue::Place(card) => place(place_shift(card)),
        Cue::TrickTake => vec![Voice::Noise {
            duration: 0.26,
            gain: 0.13,
            filter: Filter::BandPass,
            freq: 3200.0,
            sweep_to: Some(500.0),
            q: 0.8,
            start: 0.0,
        }],
        Cue::TrumpCalled(suit) => vec![with(
            tone(Wave::Triangle, trump_freq(suit), 0.3, 0.24),
            |a, _, o| {
                *a = 0.006;
                *o = true;
            },
        )],
        Cue::Euchre => vec![
            over(tone(Wave::Sine, 440.0, 0.22, 0.22)),
            over(at(
                tone(Wave::Sine, semitones(440.0, -3.0), 0.24, 0.22),
                0.14,
            )),
        ],
        Cue::March => [349.23, 440.0, 523.25, 659.25]
            .iter()
            .enumerate()
            .map(|(i, &f)| over(at(tone(Wave::Triangle, f, 0.16, 0.2), i as f32 * 0.08)))
            .collect(),
        Cue::HandWon => [523.25, 659.25, 783.99]
            .iter()
            .enumerate()
            .map(|(i, &f)| {
                with(tone(Wave::Sine, f, 0.55, 0.14), |a, s, _| {
                    *a = 0.02;
                    *s = i as f32 * 0.05;
                })
            })
            .collect(),
        Cue::GameWon => {
            let mut v: Vec<Voice> = [(392.0, 0.0), (493.88, 0.12), (587.33, 0.24), (783.99, 0.4)]
                .iter()
                .map(|&(f, s)| over(at(tone(Wave::Triangle, f, 0.5, 0.2), s)))
                .collect();
            v.extend(
                [392.0, 493.88, 587.33, 783.99]
                    .iter()
                    .map(|&f| at(tone(Wave::Sine, f, 0.7, 0.12), 0.55)),
            );
            v
        }
        Cue::Error => vec![noise(0.08, 0.16, Filter::LowPass, 550.0, 0.4)],
    }
}

/// The place thud shifted by `shift` semitones (06 §15.2 `playPlace`).
pub fn place(shift: f32) -> Vec<Voice> {
    vec![
        with(
            tone(Wave::Triangle, semitones(150.0, shift), 0.12, 0.22),
            |a, _, _| *a = 0.004,
        ),
        noise(0.07, 0.05, Filter::LowPass, 900.0, 0.9),
    ]
}

/// The cloth brush of a hover (06 §15.2 `playLift`; unused on the live table, as on the web).
pub fn lift() -> Vec<Voice> {
    vec![noise(0.09, 0.045, Filter::HighPass, 3800.0, 0.5)]
}

/// `voices` mixed into one mono buffer at [`SAMPLE_RATE`], clamped to ±1. Deterministic: the
/// noise comes from a fixed-seed generator.
pub fn render(voices: &[Voice]) -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    let end = voices
        .iter()
        .map(|v| match *v {
            Voice::Tone {
                duration, start, ..
            }
            | Voice::Noise {
                duration, start, ..
            } => start + duration,
        })
        .fold(0.0f32, f32::max);
    let mut out = vec![0.0f32; (end * rate).ceil() as usize];
    for (n, voice) in voices.iter().enumerate() {
        match *voice {
            Voice::Tone {
                wave,
                freq,
                duration,
                gain,
                attack,
                start,
                overtone,
            } => {
                add_tone(&mut out, wave, freq, start, attack, duration, gain);
                if overtone {
                    add_tone(
                        &mut out,
                        wave,
                        freq * 2.0,
                        start,
                        attack,
                        duration * 0.7,
                        gain * 0.35,
                    );
                }
            }
            Voice::Noise {
                duration,
                gain,
                filter,
                freq,
                sweep_to,
                q,
                start,
            } => add_noise(
                &mut out,
                NoiseSpec {
                    filter,
                    freq,
                    sweep_to,
                    q,
                    start,
                    duration,
                    gain,
                    seed: 0x9E37_79B9 ^ n as u32,
                },
            ),
        }
    }
    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}

/// The envelope at `t` seconds into a voice: a linear ramp from 0 to `gain` over `attack`,
/// then an exponential ramp to [`FLOOR`] at `duration`, then silence.
pub fn envelope(t: f32, attack: f32, duration: f32, gain: f32) -> f32 {
    if t < 0.0 || t >= duration {
        0.0
    } else if t < attack {
        gain * t / attack
    } else {
        let span = (duration - attack).max(f32::EPSILON);
        gain * (FLOOR / gain).powf((t - attack) / span)
    }
}

fn add_tone(
    out: &mut [f32],
    wave: Wave,
    freq: f32,
    start: f32,
    attack: f32,
    duration: f32,
    gain: f32,
) {
    let rate = SAMPLE_RATE as f32;
    let first = (start * rate).round() as usize;
    let len = (duration * rate).ceil() as usize;
    for i in 0..len {
        let Some(slot) = out.get_mut(first + i) else {
            break;
        };
        let t = i as f32 / rate;
        let phase = (freq * t).fract();
        let sample = match wave {
            Wave::Sine => (phase * std::f32::consts::TAU).sin(),
            // Starts at 0 and rises, as an oscillator does.
            Wave::Triangle => 1.0 - 4.0 * ((phase + 0.25).fract() - 0.5).abs(),
        };
        *slot += sample * envelope(t, attack, duration, gain);
    }
}

struct NoiseSpec {
    filter: Filter,
    freq: f32,
    sweep_to: Option<f32>,
    q: f32,
    start: f32,
    duration: f32,
    gain: f32,
    seed: u32,
}

fn add_noise(out: &mut [f32], spec: NoiseSpec) {
    let rate = SAMPLE_RATE as f32;
    let first = (spec.start * rate).round() as usize;
    let len = (spec.duration * rate).ceil() as usize;
    let mut rng = spec.seed | 1;
    let mut biquad = Biquad::default();
    for i in 0..len {
        let Some(slot) = out.get_mut(first + i) else {
            break;
        };
        let t = i as f32 / rate;
        // A sweep re-tunes every 32 samples, as smooth as the ear needs.
        if i % 32 == 0 {
            let f = match spec.sweep_to {
                Some(to) => spec.freq * (to.max(1.0) / spec.freq).powf(t / spec.duration),
                None => spec.freq,
            };
            biquad.tune(spec.filter, f, spec.q);
        }
        // xorshift32: deterministic white noise in −1..1.
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let white = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
        *slot += biquad.run(white) * envelope(t, NOISE_ATTACK, spec.duration, spec.gain);
    }
}

/// An RBJ biquad in transposed direct form II, with WebAudio's coefficient formulas.
#[derive(Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn tune(&mut self, filter: Filter, freq: f32, q: f32) {
        let rate = SAMPLE_RATE as f32;
        let w0 = std::f32::consts::TAU * freq.clamp(1.0, rate * 0.49) / rate;
        let (sin, cos) = w0.sin_cos();
        let (b, a0, a1, a2) = match filter {
            Filter::BandPass => {
                let alpha = sin / (2.0 * q.max(1e-4));
                ([alpha, 0.0, -alpha], 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Filter::LowPass | Filter::HighPass => {
                let alpha = sin / (2.0 * 10f32.powf(q / 20.0));
                let b = if filter == Filter::LowPass {
                    [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
                } else {
                    [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0]
                };
                (b, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
        };
        self.b = [b[0] / a0, b[1] / a0, b[2] / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Whether this process is the app in a real window: the `euchre` binary, not headless. Only
/// then does a cue open a speaker or the tour open by itself; headless runs (`--snap`,
/// `--frames`, `--trace`) and the test binaries never do.
pub fn live_app() -> bool {
    !rune::is_headless() && is_app_binary()
}

/// Whether the running executable is the `euchre` app (or its bundled `Euchre`), not a test
/// binary (`coach-…`, `motion-…`, `euchre_desktop-…`).
pub fn is_app_binary() -> bool {
    // A page has no executable: the browser build is always the app.
    if cfg!(target_arch = "wasm32") {
        return true;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .is_some_and(|stem| stem.eq_ignore_ascii_case("euchre"))
}

/// The output's state.
#[derive(Debug, Default)]
enum Output {
    /// Not opened yet: nothing has played.
    #[default]
    Closed,
    /// The audio thread is up; cues go to it.
    #[cfg(not(target_arch = "wasm32"))]
    Open(Sender<Cue>),
    /// The page's WebAudio context is in use (it lives in a thread local: it is not `Send`).
    #[cfg(target_arch = "wasm32")]
    Open,
    /// No device, or this process may not open one: silent for the run.
    Off,
}

/// The sound player.
#[derive(Debug, Default)]
pub struct Sound {
    enabled: bool,
    out: Output,
}

impl Sound {
    /// A muted player. Opens nothing.
    pub fn new() -> Sound {
        Sound::default()
    }

    /// Turn the player on or off (the Sound preference). Turning it on opens nothing; the first
    /// cue does.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Whether the player is on.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether an output was opened (for tests: it stays `false` outside the app window).
    pub fn opened(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            matches!(self.out, Output::Open(_))
        }
        #[cfg(target_arch = "wasm32")]
        {
            matches!(self.out, Output::Open)
        }
    }

    /// Play `cue` if the player is on, opening the output on the first one.
    pub fn play(&mut self, cue: &Cue) {
        if !self.enabled {
            return;
        }
        if matches!(self.out, Output::Closed) {
            self.out = if live_app() { open() } else { Output::Off };
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Output::Open(tx) = &self.out
            && tx.send(*cue).is_err()
        {
            // The thread found no device (it said so once) and ended.
            self.out = Output::Off;
        }
        #[cfg(target_arch = "wasm32")]
        if matches!(self.out, Output::Open) && !web_audio::play(cue) {
            tracing::warn!("no WebAudio output; the table stays silent");
            self.out = Output::Off;
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn open() -> Output {
    Output::Open
}

/// The same rendered cues, played through the page's `AudioContext` (W-14). The context is made
/// on the first cue, which comes after the click that turned sound on, so the page has the user
/// activation the autoplay rules want; a suspended context is resumed on each cue.
#[cfg(target_arch = "wasm32")]
mod web_audio {
    use std::cell::RefCell;

    use super::{Cue, SAMPLE_RATE, recipe, render};

    type Cache = Vec<(Cue, web_sys::AudioBuffer)>;

    thread_local! {
        static OUT: RefCell<Option<(web_sys::AudioContext, Cache)>> = const { RefCell::new(None) };
    }

    /// Plays `cue`; false when the page has no audio output.
    pub fn play(cue: &Cue) -> bool {
        OUT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some((web_sys::AudioContext::new().ok()?, Vec::new()));
            }
            let (ctx, cache) = slot.as_mut()?;
            if ctx.state() == web_sys::AudioContextState::Suspended {
                let _ = ctx.resume();
            }
            let buffer = match cache.iter().find(|(c, _)| c == cue) {
                Some((_, b)) => b.clone(),
                None => {
                    let samples = render(&recipe(cue));
                    let len = u32::try_from(samples.len()).ok()?.max(1);
                    let b = ctx.create_buffer(1, len, SAMPLE_RATE as f32).ok()?;
                    b.copy_to_channel(&samples, 0).ok()?;
                    cache.push((*cue, b.clone()));
                    b
                }
            };
            let source = ctx.create_buffer_source().ok()?;
            source.set_buffer(Some(&buffer));
            source.connect_with_audio_node(&ctx.destination()).ok()?;
            source.start().ok()?;
            Some(())
        })
        .is_some()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open() -> Output {
    let (tx, rx) = mpsc::channel();
    match std::thread::Builder::new()
        .name("euchre-sound".to_owned())
        .spawn(move || audio_thread(rx))
    {
        Ok(_) => Output::Open(tx),
        Err(err) => {
            tracing::warn!(%err, "sound thread did not start; the table stays silent");
            Output::Off
        }
    }
}

/// Holds the device and plays each cue it receives, rendering each once.
#[cfg(not(target_arch = "wasm32"))]
fn audio_thread(rx: Receiver<Cue>) {
    let mut sink = match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => sink,
        Err(err) => {
            tracing::warn!(%err, "no audio output; the table stays silent");
            return;
        }
    };
    sink.log_on_drop(false);
    let (Some(mono), Some(rate)) = (NonZero::new(1u16), NonZero::new(SAMPLE_RATE)) else {
        return;
    };
    let mut cache: Vec<(Cue, rodio::buffer::SamplesBuffer)> = Vec::new();
    while let Ok(cue) = rx.recv() {
        let buffer = match cache.iter().find(|(c, _)| *c == cue) {
            Some((_, b)) => b.clone(),
            None => {
                let b = rodio::buffer::SamplesBuffer::new(mono, rate, render(&recipe(&cue)));
                cache.push((cue, b.clone()));
                b
            }
        };
        sink.mixer().add(buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_player_is_muted_and_plays_nothing() {
        let mut s = Sound::new();
        assert!(!s.enabled());
        s.play(&Cue::Deal);
        assert!(!s.opened());
        s.set_enabled(true);
        assert!(s.enabled());
        s.play(&Cue::Deal);
        assert!(!s.opened(), "a test binary never opens a speaker");
    }

    #[test]
    fn the_envelope_ramps_then_decays_to_the_floor() {
        assert_eq!(envelope(0.0, 0.01, 0.2, 0.5), 0.0);
        assert!((envelope(0.01, 0.01, 0.2, 0.5) - 0.5).abs() < 1e-5);
        let late = envelope(0.199_999, 0.01, 0.2, 0.5);
        assert!(late < 0.000_11, "{late}");
        assert_eq!(envelope(0.2, 0.01, 0.2, 0.5), 0.0);
    }
}
