//! Prints bounded, reproducible pre-listening measurements for the three readers and Character.
//! This is evidence for tuning provisional constants, not an owner-listening substitute.

use mxm_creative_sampler_dsp::{
    Character, Engine, GRAIN_BUDGET, LayerParams, LoopMode, MAX_OVERLAP, MAX_VOICES, Note, Params,
    Reader, SINC_TAPS, STRETCH_WINDOW_S, Sample,
};
use std::f32::consts::TAU;
use std::hint::black_box;
use std::time::Instant;

const RATE: f32 = 48_000.0;
const FRAMES: usize = 32_768;
const SKIP: usize = 4_096;

fn sample() -> Sample {
    Sample::new(
        (0..48_000)
            .map(|n| {
                let t = n as f32 / RATE;
                let chirp = (TAU * (180.0 * t + 5_200.0 * t * t)).sin();
                let transient = if n % 4_000 < 80 {
                    1.0 - (n % 4_000) as f32 / 80.0
                } else {
                    0.0
                };
                let value = chirp * 0.45 + transient * 0.35;
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap()
}

fn alias_probe() -> Sample {
    Sample::new(
        (0..48_000)
            .map(|n| {
                let t = n as f32 / RATE;
                let value = 0.45 * (TAU * 2_000.0 * t).sin() + 0.45 * (TAU * 18_000.0 * t).sin();
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap()
}

fn render(source: &Sample, reader: Reader, character: Character, key: u8) -> Vec<f32> {
    let mut params = Params::default();
    params.layers[0].reader = reader;
    params.layers[0].loop_mode = LoopMode::Forward;
    params.character = character;
    let mut engine = Engine::new(RATE);
    engine.note_on(Note::midi(key, 1.0), [Some(source), None], &params);
    (0..FRAMES)
        .map(|_| engine.render([Some(source), None], &params).left)
        .skip(SKIP)
        .collect()
}

fn magnitude_at(signal: &[f32], hz: f32) -> f64 {
    let mut real = 0.0f64;
    let mut imaginary = 0.0f64;
    for (index, sample) in signal.iter().enumerate() {
        let phase = f64::from(TAU * hz * index as f32 / RATE);
        real += f64::from(*sample) * phase.cos();
        imaginary -= f64::from(*sample) * phase.sin();
    }
    2.0 * real.hypot(imaginary) / signal.len() as f64
}

fn rms(signal: &[f32]) -> f64 {
    (signal
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / signal.len() as f64)
        .sqrt()
}

fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let dot = a
        .iter()
        .zip(b)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum::<f64>();
    let norm = a
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt()
        * b.iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt();
    dot / norm.max(f64::MIN_POSITIVE)
}

fn main() {
    let probe = alias_probe();
    let repitch = render(&probe, Reader::Repitch, Character::default(), 72);
    let wanted = magnitude_at(&repitch, 4_000.0);
    let rejected_alias = magnitude_at(&repitch, 12_000.0);
    println!(
        "repitch +12 st: wanted 4 kHz {:.6}, rejected 18→12 kHz alias {:.6} ({:.1} dB relative)",
        wanted,
        rejected_alias,
        20.0 * (rejected_alias / wanted.max(f64::MIN_POSITIVE)).log10()
    );

    // **The unity-rate path has a different job from the alias one.** At or below the recorded
    // rate there is no resampling alias to reject; the kernel is a fractional-delay interpolator
    // and what it can lose is the top of the passband. One semitone down sweeps the fractional
    // position through every phase of the table while keeping the band limit wide open.
    let down = render(&probe, Reader::Repitch, Character::default(), 59);
    let ratio = 2.0f32.powf(-1.0 / 12.0);
    println!(
        "unity path at -1 st ({} taps): 2 kHz kept {:.6}, 18 kHz kept {:.6} ({:.2} dB of top-octave droop)",
        SINC_TAPS,
        magnitude_at(&down, 2_000.0 * ratio),
        magnitude_at(&down, 18_000.0 * ratio),
        20.0 * (magnitude_at(&down, 18_000.0 * ratio)
            / magnitude_at(&down, 2_000.0 * ratio).max(f64::MIN_POSITIVE))
        .log10()
    );

    let source = sample();
    // At the root note Repitch and Stretch should agree: neither needs to change duration. One
    // octave up is the distinguishing case: Repitch runs through the source twice as fast while
    // Stretch keeps the travel duration and raises only its within-window reads.
    let clean = render(&source, Reader::Repitch, Character::default(), 72);
    let stretch = render(&source, Reader::Stretch, Character::default(), 72);
    let grain = render(&source, Reader::Grain, Character::default(), 72);
    println!(
        "reader correlation: repitch/stretch {:.4}, repitch/grain {:.4}, stretch/grain {:.4}",
        correlation(&clean, &stretch),
        correlation(&clean, &grain),
        correlation(&stretch, &grain)
    );

    let character_clean = render(&source, Reader::Repitch, Character::default(), 60);
    let degraded = render(
        &source,
        Reader::Repitch,
        Character {
            rate: 1.0,
            converter: 1.0,
            reconstruction: 1.0,
            input: 1.0,
            ..Character::default()
        },
        60,
    );
    println!(
        "Character clean/max: correlation {:.4}, RMS {:.5}/{:.5}",
        correlation(&character_clean, &degraded),
        rms(&character_clean),
        rms(&degraded)
    );

    // The cloud's own laws, measured rather than asserted: what density does to weight, and what
    // Spread does that Scatter no longer does for it.
    let cloud = |size_s: f32, density: f32, scatter: f32, spread: f32| {
        let mut params = Params {
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_size_s: size_s,
            grain_density_hz: density,
            grain_position_variation: scatter,
            grain_onset_timing: scatter,
            grain_spread_semitones: spread,
            ..LayerParams::default()
        };
        let mut engine = Engine::new(RATE);
        engine.note_on(Note::midi(60, 1.0), [Some(&source), None], &params);
        for _ in 0..4_800 {
            engine.render([Some(&source), None], &params);
        }
        let rendered: Vec<f32> = (0..48_000)
            .map(|_| engine.render([Some(&source), None], &params).left)
            .collect();
        let peak = rendered.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        (rms(&rendered), f64::from(peak))
    };
    for (label, scatter) in [("coherent", 0.0), ("scattered", 1.0)] {
        let (thin, thin_peak) = cloud(0.06, 16.0, scatter, 0.0);
        let (thick, thick_peak) = cloud(0.06, 64.0, scatter, 0.0);
        println!(
            "grain overlap law, {label}: RMS {thin:.5}→{thick:.5} ({:.2}×) as overlap goes ~1→{:.1}, peak {thin_peak:.3}→{thick_peak:.3}",
            thick / thin.max(1.0e-12),
            (0.06 * 64.0f32).min(MAX_OVERLAP),
        );
    }
    // **The peak sweep that fixes the ceiling.** There is no limiter in this instrument, only a
    // clip indicator, so the laws themselves have to land under full scale everywhere the controls
    // reach. A stochastic cloud's crest factor grows with the count, so RMS compensation alone does
    // not bound the peak — this is the measurement that says by how much.
    let mut worst = 0.0f64;
    let mut worst_at = (0.0f32, 0.0f32, 0.0f32);
    for scatter in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let mut row = String::new();
        for (size, density) in [
            (0.008f32, 16.0f32),
            (0.06, 16.0),
            (0.06, 64.0),
            (0.12, 128.0),
            (0.25, 200.0),
            (1.0, 200.0),
        ] {
            let (_, peak) = cloud(size, density, scatter, 0.0);
            let overlap = (size * density).clamp(1.0, MAX_OVERLAP);
            if peak > worst {
                worst = peak;
                worst_at = (scatter, size, density);
            }
            row.push_str(&format!(" ov{overlap:.1}:{peak:.3}"));
        }
        println!("grain peak sweep, scatter {scatter:.2}:{row}");
    }
    println!(
        "grain peak sweep: worst {worst:.3} at scatter {:.2}, size {:.3} s, density {:.0}/s",
        worst_at.0, worst_at.1, worst_at.2
    );

    let (plain, _) = cloud(0.06, 32.0, 0.0, 0.0);
    let (detuned, _) = cloud(0.06, 32.0, 0.0, 3.0);
    println!(
        "grain Spread at zero Scatter: RMS {plain:.5}→{detuned:.5} — detune is independent of scatter"
    );

    let mut dense = Params::default();
    for layer in &mut dense.layers {
        layer.reader = Reader::Grain;
        layer.loop_mode = LoopMode::Forward;
        layer.grain_density_hz = 100.0;
        layer.grain_size_s = 0.25;
        layer.grain_position_variation = 1.0;
        layer.grain_onset_timing = 1.0;
    }
    let mut engine = Engine::new(RATE);
    for note in 48..48 + MAX_VOICES as u8 {
        engine.note_on(
            Note::midi(note, 1.0),
            [Some(&source), Some(&source)],
            &dense,
        );
    }
    let rendered_frames = 240_000usize;
    let started = Instant::now();
    let mut checksum = 0.0f32;
    for _ in 0..rendered_frames {
        let value = engine.render([Some(&source), Some(&source)], &dense);
        checksum += value.left + value.right;
    }
    black_box(checksum);
    let elapsed = started.elapsed().as_secs_f64();
    let audio_seconds = rendered_frames as f64 / f64::from(RATE);
    // Count the grains rather than inferring them. The budget is shared, so `voices × layers ×
    // pool` is no longer the live total, and the whole point of the probe is what it actually cost.
    println!(
        "dense CPU probe: {MAX_VOICES} voices × 2 layers, {live} of {GRAIN_BUDGET} grains live, {:.3} s for {:.1} s audio ({:.1}% of one core realtime)",
        elapsed,
        audio_seconds,
        elapsed / audio_seconds * 100.0,
        live = engine.active_grains(),
    );

    measure_stretch();
    measure_crossfade();
}

// ---------------------------------------------------------------------------------------------
// S1: the waveform-aligned splice. plans/plan-stretch-quality.md
// ---------------------------------------------------------------------------------------------

/// A steady, harmonically rich tone, so any modulation in the output came from the reader.
///
/// **A single sine is a degenerate probe for this and the first version of the measurement used
/// one.** Two heads reading one sine at one rate sum to a sine; the splice changes its amplitude
/// by a fixed amount every window and the flutter all but vanishes. A comb needs partials to comb:
/// an unaligned splice cancels each harmonic by a different amount, which is what the artefact
/// actually sounds like and what this source is for.
fn steady_tone(hz: f32) -> Sample {
    Sample::new(
        (0..96_000)
            .map(|n| {
                let t = n as f32 / RATE;
                let mut value = 0.0;
                for harmonic in 1..=12 {
                    value += (TAU * hz * harmonic as f32 * t).sin() / harmonic as f32;
                }
                let value = value * 0.25;
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap()
}

/// A struck source: four attacks over a quiet bed, for the transient measurement.
fn struck_source() -> Sample {
    let onsets = [12_000usize, 30_000, 48_000, 66_000];
    let mut frames = vec![[0.0f32; 2]; 96_000];
    for (n, frame) in frames.iter_mut().enumerate() {
        let bed = (TAU * 196.0 * n as f32 / RATE).sin() * 0.06;
        frame[0] = bed;
        frame[1] = bed;
    }
    for onset in onsets {
        for n in 0..(RATE * 0.05) as usize {
            let index = onset + n;
            if index >= frames.len() {
                break;
            }
            let decay = (-(n as f32) / (RATE * 0.006)).exp();
            let strike = (TAU * 1_500.0 * n as f32 / RATE).sin() * decay * 0.8;
            frames[index][0] += strike;
            frames[index][1] += strike;
        }
    }
    Sample::new(frames, RATE).unwrap()
}

/// **The reference this change is measured against: the splice the shipped reader used to make.**
///
/// Two heads, one fixed window, each re-anchoring to the playhead the moment its age runs out —
/// no correlation search and no onset map. It is implemented here rather than kept in the crate
/// because `docs/oscillators/AGENTS.md`'s rule is the right one: a candidate that is not shipped
/// lives in the harness, so the two are measured through the same code path.
///
/// Linear interpolation on both sides, deliberately. The shipped reader interpolates with a
/// sixteen-tap windowed sinc and this one cannot, so matching the *interpolator* would confuse the
/// comparison; what is being compared is the **splice**, and both sides splice identically apart
/// from the alignment.
fn reference_ola(source: &Sample, semitones: f32, aligned: bool, frames: usize) -> Vec<f32> {
    let window = (STRETCH_WINDOW_S * RATE).round() as usize;
    let pitch = 2.0f32.powf(semitones / 12.0);
    let len = source.len() as f32;
    let read = |position: f32| -> f32 {
        let wrapped = position.rem_euclid(len);
        let index = wrapped.floor() as usize;
        let frac = wrapped - index as f32;
        let a = source.frames()[index % source.len()][0];
        let b = source.frames()[(index + 1) % source.len()][0];
        a + (b - a) * frac
    };
    // The same normalised cross-correlation the shipped search uses, at the same span and range.
    let align = |reference: f32, candidate: f32| -> f32 {
        if !aligned {
            return 0.0;
        }
        let search = (0.010 * RATE) as i32;
        let points = 64usize;
        let dt = (0.020 * RATE) / points as f32;
        let mut best = 0i32;
        let mut best_score = f32::NEG_INFINITY;
        let score_of = |offset: i32| {
            let (mut ab, mut aa, mut bb) = (0.0f32, 0.0f32, 0.0f32);
            for point in 0..points {
                let walk = point as f32 * dt * pitch;
                let a = read(reference + walk);
                let b = read(candidate + offset as f32 + walk);
                ab += a * b;
                aa += a * a;
                bb += b * b;
            }
            let denominator = (aa * bb).sqrt();
            if denominator > 1.0e-12 {
                ab / denominator
            } else {
                0.0
            }
        };
        let mut offset = 0i32;
        while offset <= search {
            for candidate_offset in if offset == 0 {
                vec![0]
            } else {
                vec![offset, -offset]
            } {
                let score = score_of(candidate_offset);
                if score > best_score + 1.0e-3 {
                    best_score = score;
                    best = candidate_offset;
                }
            }
            offset += 16;
        }
        best as f32
    };

    let mut travel = 0.0f32;
    let mut position = [0.0f32, 0.0f32];
    let mut age = [0usize, window / 2];
    let mut out = Vec::with_capacity(frames);
    for _ in 0..frames {
        for head in 0..2 {
            if age[head] >= window {
                let offset = align(position[1 - head], travel);
                age[head] = 0;
                position[head] = travel + offset;
            }
        }
        let mut value = 0.0f32;
        let mut weight = 0.0f32;
        for head in 0..2 {
            let phase = age[head] as f32 / window as f32;
            let w = 0.5 - 0.5 * (TAU * phase).cos();
            value += read(position[head]) * w;
            weight += w;
            position[head] += pitch;
            age[head] += 1;
        }
        out.push(if weight > 1.0e-3 { value / weight } else { 0.0 });
        travel += 1.0;
    }
    out
}

/// **The complaint, turned into a number: splice sidebands.**
///
/// An unaligned splice cancels each partial by a different amount, and it does so afresh every
/// window — so every harmonic acquires sidebands spaced at the window rate. Their level relative
/// to the harmonics they surround is the flam, the chorusing and the metallic edge, and it is what
/// aligning the splice is supposed to remove.
///
/// **Two earlier versions of this measurement were wrong and both flattered the result.** Measuring
/// amplitude modulation depth instead: a deterministic splice on a periodic source repeats its
/// phase relationship exactly every window, so the artefact is a comb that is *constant in time*
/// and carries almost no AM — the metric read near zero for a clearly audible defect. And measuring
/// without a window function: the carrier is 40 dB above its own leakage eighteen bins out, which
/// is the same order as the sidebands being looked for. A Hann window fixes the second.
fn splice_sidebands_db(signal: &[f32], fundamental: f32) -> f64 {
    let windowed: Vec<f32> = signal
        .iter()
        .enumerate()
        .map(|(n, value)| {
            let phase = TAU * n as f32 / signal.len() as f32;
            value * (0.5 - 0.5 * phase.cos())
        })
        .collect();
    let spacing = 1.0 / STRETCH_WINDOW_S;
    let mut carrier = 0.0f64;
    let mut sidebands = 0.0f64;
    for harmonic in 1..=8 {
        let centre = fundamental * harmonic as f32;
        if centre > 16_000.0 {
            break;
        }
        carrier += magnitude_at(&windowed, centre).powi(2);
        for order in 1..=2 {
            let offset = order as f32 * spacing;
            sidebands += magnitude_at(&windowed, centre - offset).powi(2);
            sidebands += magnitude_at(&windowed, centre + offset).powi(2);
        }
    }
    10.0 * (sidebands / carrier.max(f64::MIN_POSITIVE)).log10()
}

/// The shipped reader with the amplitude envelope held flat and the attack skipped.
///
/// **The envelope is not the reader.**  decays to a 0.82 sustain over the first
/// part of a note, and a decay ramp has energy at every low frequency including the window rate —
/// so the first version of this measurement reported 0.0885 for the shipped reader at both +7 and
/// +12 st, a number that did not move with the interval because it was the envelope and not the
/// splice.
fn render_flat(source: &Sample, reader: Reader, key: u8, frames: usize) -> Vec<f32> {
    let mut params = Params::default();
    params.layers[0].reader = reader;
    params.layers[0].loop_mode = LoopMode::Forward;
    params.attack_s = 0.001;
    params.decay_s = 0.001;
    params.sustain = 1.0;
    let mut engine = Engine::new(RATE);
    engine.note_on(Note::midi(key, 1.0), [Some(source), None], &params);
    for _ in 0..SKIP {
        engine.render([Some(source), None], &params);
    }
    (0..frames)
        .map(|_| engine.render([Some(source), None], &params).left)
        .collect()
}

/// The sharpest level rise anywhere in a signal, over a 2 ms span, **as a multiple of the
/// signal's own RMS**.
///
/// The normalisation is what makes the number comparable across signals. A rendered voice carries
/// the master gain and an equal-power pan that the source does not, so the raw rises are on
/// different scales and comparing them directly says more about gain staging than about smearing.
fn sharpest_rise(signal: &[f32]) -> f64 {
    let level = rms(signal);
    if level <= f64::MIN_POSITIVE {
        return 0.0;
    }
    raw_rise(signal) / level
}

fn raw_rise(signal: &[f32]) -> f64 {
    let span = (RATE * 0.002) as usize;
    let envelope: Vec<f32> = signal.iter().map(|value| value.abs()).collect();
    let mut sharpest = 0.0f64;
    let mut n = span;
    while n + span < envelope.len() {
        let before = envelope[n - span..n]
            .iter()
            .map(|v| f64::from(*v))
            .sum::<f64>()
            / span as f64;
        let after = envelope[n..n + span]
            .iter()
            .map(|v| f64::from(*v))
            .sum::<f64>()
            / span as f64;
        sharpest = sharpest.max(after - before);
        n += span / 2;
    }
    sharpest
}

/// A tone whose timing is *not* stationary: harmonics of `hz` under a deep 3 Hz tremolo.
///
/// **A steady tone cannot grade a time-stretcher.** A correctly aligned overlap-add is transparent
/// on one, which is why every reading on the steady sources sits at the metric's floor whatever the
/// window does. Repeating or skipping a few milliseconds of a *steady* tone is inaudible by
/// construction; doing it to a tone whose envelope is moving is exactly the granular chatter the
/// owner reported, and this source is built so that it shows.
fn moving_tone(hz: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            let t = n as f32 / RATE;
            let mut value = 0.0;
            for harmonic in 1..=12 {
                value += (TAU * hz * harmonic as f32 * t).sin() / harmonic as f32;
            }
            value * 0.25 * (1.0 - 0.9 * (TAU * 3.0 * t).cos()) * 0.5
        })
        .collect()
}

/// How far a rendered signal's **time-frequency envelope** is from the ideal's, in dB.
///
/// **Phase-blind on purpose, and that is the whole point.** Comparing waveforms sample by sample
/// scores two identical-sounding tones a quarter-cycle apart as total error — the first version of
/// this measurement reported -1 to -3 dB for a reader whose output was perceptually right. What a
/// listener hears is *which harmonics are present, how loud, and when*, so that is what is
/// compared: the magnitude at each of the first six harmonics, over short overlapping frames,
/// against the same from the synthesised ideal. Timing errors and window chatter move it; a phase
/// offset does not.
fn envelope_error_db(rendered: &[f32], ideal: &[f32], fundamental: f32) -> f64 {
    const FRAME: usize = 2_048;
    let frames = rendered.len().min(ideal.len());
    if frames < FRAME * 2 {
        return 0.0;
    }
    let harmonics: Vec<f32> = (1..=6)
        .map(|h| fundamental * h as f32)
        .filter(|hz| *hz < RATE * 0.45)
        .collect();
    let mut error = 0.0f64;
    let mut signal = 0.0f64;
    let mut at = 0usize;
    while at + FRAME <= frames {
        let window: Vec<f32> = (0..FRAME)
            .map(|n| 0.5 - 0.5 * (TAU * n as f32 / FRAME as f32).cos())
            .collect();
        let a: Vec<f32> = rendered[at..at + FRAME]
            .iter()
            .zip(&window)
            .map(|(v, w)| v * w)
            .collect();
        let b: Vec<f32> = ideal[at..at + FRAME]
            .iter()
            .zip(&window)
            .map(|(v, w)| v * w)
            .collect();
        // Level is not an error; the reader carries the instrument's gain staging.
        let scale = {
            let ra: f64 = a.iter().map(|v| f64::from(*v).powi(2)).sum();
            let rb: f64 = b.iter().map(|v| f64::from(*v).powi(2)).sum();
            if rb > f64::MIN_POSITIVE {
                (ra / rb).sqrt()
            } else {
                0.0
            }
        };
        for hz in &harmonics {
            let ma = magnitude_at(&a, *hz);
            let mb = magnitude_at(&b, *hz) * scale;
            error += (ma - mb).powi(2);
            signal += mb.powi(2);
        }
        at += FRAME / 2;
    }
    10.0 * (error / signal.max(f64::MIN_POSITIVE)).log10()
}

fn measure_stretch() {
    println!();

    // **What the onset map costs at load**, because analysis now completes before a sample is
    // published and that latency is the player's to accept. Timed over a realistic drop rather
    // than the probe's own short sources.
    for seconds in [1.0f32, 5.0, 30.0] {
        let frames: Vec<[f32; 2]> = (0..(seconds * RATE) as usize)
            .map(|n| {
                let t = n as f32 / RATE;
                let value =
                    (TAU * 220.0 * t).sin() * 0.4 + if n % 12_000 < 400 { 0.5 } else { 0.0 };
                [value, value]
            })
            .collect();
        let started = Instant::now();
        let built = Sample::new(frames, RATE).unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        println!(
            "sample preparation: {seconds:.0} s stereo source analysed in {:.1} ms, {} onsets",
            elapsed * 1_000.0,
            built.transients().len()
        );
    }

    let tone = steady_tone(220.0);

    // The reference pair first, because it is the controlled comparison: one implementation, one
    // difference.
    for (fundamental, semitones) in [
        (220.0f32, 5.0f32),
        (220.0, 7.0),
        (220.0, 12.0),
        (55.0, 1.0),
        (55.0, 3.0),
        (55.0, 7.0),
    ] {
        let tone = steady_tone(fundamental);
        let pitch = 2.0f32.powf(semitones / 12.0);
        let clocked = reference_ola(&tone, semitones, false, 69_120);
        let aligned = reference_ola(&tone, semitones, true, 69_120);
        let a = splice_sidebands_db(&clocked, fundamental * pitch);
        let b = splice_sidebands_db(&aligned, fundamental * pitch);
        println!(
            "splice sidebands at +{semitones:.0} st on {fundamental:.0} Hz, reference OLA: clocked {a:.1} dB, aligned {b:.1} dB ({:.1} dB better)",
            a - b
        );
    }

    // Then the shipped reader, with a flat envelope so what is measured is the splice.
    // **The owner plays a few notes up and down, so that is where this has to be measured.**
    // The first sweep stopped at +5 st and the report came back from inside it.
    // **Two fundamentals, because the window length trades against the lowest note.** A window has
    // to hold enough periods of the source for the search to align it: 220 Hz forgives a short one
    // and a bass note does not, so a figure taken on one pitch alone would tune the reader for half
    // the keyboard and call it done.
    for (label, fundamental) in [
        ("55 Hz", 55.0f32),
        ("110 Hz", 110.0),
        ("220 Hz", 220.0),
        ("440 Hz", 440.0),
    ] {
        let source = steady_tone(fundamental);
        let mut row = String::new();
        let mut worst = 0.0f64;
        for semitones in [
            -7.0f32, -5.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 5.0, 7.0,
        ] {
            let key = (60.0 + semitones) as u8;
            let pitch = 2.0f32.powf(semitones / 12.0);
            let rendered = render_flat(&source, Reader::Stretch, key, 69_120);
            let db = splice_sidebands_db(&rendered, fundamental * pitch);
            if semitones != 0.0 {
                worst = worst.min(-db);
            }
            row.push_str(&format!(" {semitones:+.0}:{db:.0}"));
        }
        println!(
            "splice sidebands, {label} source (dB):{row} | worst {:.0}",
            -worst
        );
        // **The control, which should have come first.** Repitch splices nothing, so whatever it
        // reports at this fundamental is the metric's own floor rather than the reader's artefact.
        let mut control = String::new();
        for semitones in [-3.0f32, -1.0, 0.0, 1.0, 3.0] {
            let key = (60.0 + semitones) as u8;
            let pitch = 2.0f32.powf(semitones / 12.0);
            let rendered = render_flat(&source, Reader::Repitch, key, 69_120);
            control.push_str(&format!(
                " {semitones:+.0}:{:.0}",
                splice_sidebands_db(&rendered, fundamental * pitch)
            ));
        }
        println!("  metric floor, {label} via Repitch (dB):{control}");
        println!(
            "  onsets {} (expected 0), period track at 1 s: {:?} frames (expected {:.0})",
            source.transients().len(),
            source.period_at(48_000.0).map(|p| p.round()),
            RATE / fundamental
        );
    }
    println!(
        "onsets the detector reports on that steady tone: {}",
        tone.transients().len()
    );

    // **The gate metric: error against a synthesised ground truth.** Lower is better; this one has
    // no floor to hide under.
    for hz in [55.0f32, 110.0, 220.0, 440.0] {
        let source = Sample::new(
            moving_tone(hz, 96_000)
                .into_iter()
                .map(|v| [v, v])
                .collect(),
            RATE,
        )
        .unwrap();
        let mut row = String::new();
        for semitones in [-5.0f32, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 5.0] {
            let key = (60.0 + semitones) as u8;
            let ratio = 2.0f32.powf(semitones / 12.0);
            let rendered = render_flat(&source, Reader::Stretch, key, 60_000);
            // The render skips `SKIP` frames of the source, so the ideal has to start at the same
            // instant or the tremolo is compared against itself a quarter of a cycle out — which
            // read as 1-3 dB of "error" at unity, where the reader is provably an identity.
            let ideal = moving_tone(hz * ratio, 96_000);
            row.push_str(&format!(
                " {semitones:+.0}:{:.0}",
                envelope_error_db(&rendered, &ideal[SKIP..], hz * ratio)
            ));
        }
        println!("error against ideal, {hz:.0} Hz moving tone (dB):{row}");
    }

    // Transients. The source's own attacks set the scale; a reader that smears them reports less.
    let struck = struck_source();
    println!(
        "struck source: {} onsets found at {:?}",
        struck.transients().len(),
        &struck.transients()[..struck.transients().len().min(6)]
    );
    // The control must cover the span the renders actually cover, or it is comparing different
    // parts of a source that is not stationary.
    let source_rise = sharpest_rise(
        &struck.frames()[SKIP..SKIP + 72_000]
            .iter()
            .map(|f| f[0])
            .collect::<Vec<f32>>(),
    );
    let clocked = reference_ola(&struck, 7.0, false, 72_000);
    let aligned = reference_ola(&struck, 7.0, true, 72_000);
    println!(
        "attack preservation at +7 st: source rise {source_rise:.4}, clocked splice {:.4}, aligned splice {:.4}",
        sharpest_rise(&clocked),
        sharpest_rise(&aligned),
    );
    // The shipped reader carries a band limit the reference does not: above the recorded rate its
    // kernel narrows with pitch, which is the transposition antialiasing this crate is measured on
    // and it necessarily softens an attack. Reported at unity as well, where no band limit applies,
    // so the two effects are separable rather than conflated.
    // **Averaged over successive note-ons, because the start phase is staggered per voice.** The
    // stagger exists so a chord does not re-anchor in lockstep, and it necessarily randomises which
    // window phase an attack lands on — so a single note is one draw from a distribution, and
    // reporting it as *the* figure is how a 5.70 and a 4.44 both looked like facts.
    let mut rises: Vec<f64> = Vec::new();
    for take in 0..8u64 {
        let mut params = Params::default();
        params.layers[0].reader = Reader::Stretch;
        params.layers[0].loop_mode = LoopMode::Off;
        params.attack_s = 0.001;
        params.decay_s = 0.001;
        params.sustain = 1.0;
        let mut engine = Engine::new(RATE);
        // Successive presses advance the engine's chronology, which is what seeds the stagger.
        for _ in 0..take {
            engine.note_on(Note::midi(40, 1.0), [Some(&struck), None], &params);
            engine.panic();
        }
        engine.note_on(Note::midi(67, 1.0), [Some(&struck), None], &params);
        for _ in 0..SKIP {
            engine.render([Some(&struck), None], &params);
        }
        let rendered: Vec<f32> = (0..72_000)
            .map(|_| engine.render([Some(&struck), None], &params).left)
            .collect();
        rises.push(sharpest_rise(&rendered));
    }
    let mean = rises.iter().sum::<f64>() / rises.len() as f64;
    let worst = rises.iter().cloned().fold(f64::INFINITY, f64::min);
    let best = rises.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    println!(
        "attack preservation, shipped reader at +7 st over 8 voices: mean {mean:.2}, worst {worst:.2}, best {best:.2} (unity reads {:.2}, exactly the source)",
        sharpest_rise(&render_flat(&struck, Reader::Stretch, 60, 72_000)),
    );

    // Cost. The alignment search is bursty by construction — it runs once per head per window —
    // so what matters is the worst case with every voice sounding, not the mean of a single note.
    let mut params = Params::default();
    for layer in &mut params.layers {
        layer.reader = Reader::Stretch;
        layer.loop_mode = LoopMode::Forward;
    }
    let mut engine = Engine::new(RATE);
    for note in 48..48 + MAX_VOICES as u8 {
        engine.note_on(Note::midi(note, 1.0), [Some(&tone), Some(&tone)], &params);
    }
    let rendered_frames = 240_000usize;
    let started = Instant::now();
    let mut checksum = 0.0f32;
    for _ in 0..rendered_frames {
        let value = engine.render([Some(&tone), Some(&tone)], &params);
        checksum += value.left + value.right;
    }
    black_box(checksum);
    let elapsed = started.elapsed().as_secs_f64();
    let audio_seconds = f64::from(rendered_frames as f32 / RATE);
    println!(
        "stretch CPU probe: {MAX_VOICES} voices × 2 layers aligned, {:.3} s for {:.1} s audio ({:.1}% of one core realtime)",
        elapsed,
        audio_seconds,
        elapsed / audio_seconds * 100.0,
    );
}

/// **The loop crossfade, measured**: what its gain law does to level on material that agrees at
/// both ends of the loop and on material that does not, and what the seam costs where it is widest.
///
/// `plans/plan-sampler-loop-crossfade.md` §2.3 chose an amplitude-complementary law and asked for
/// its dip on uncorrelated material to be recorded rather than assumed small. The correlated case is
/// a tone whose loop spans whole cycles, so the frame one period away is the same frame and the fade
/// should change nothing — the control. The uncorrelated case is noise, where two independent halves
/// summed at `cos²` and `sin²` lose power toward the middle of the fade.
fn measure_crossfade() {
    println!();
    // A loop of 24 000 frames with a 4 800-frame fade. The fade takes its material from inside the
    // loop, so each pass returns 4 800 frames past Loop start and lasts 19 200 frames — exactly 88
    // cycles of 220 Hz, which is what keeps the tone a loop that closes.
    const LOOP_START: usize = 12_000;
    const LOOP_END: usize = 36_000;
    const FADE: usize = 4_800;
    let len = 48_001usize;
    let last = (len - 1) as f32;
    let tone = Sample::new(
        (0..len)
            .map(|n| {
                let value = (TAU * 220.0 * n as f32 / RATE).sin() * 0.5;
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let noise = Sample::new(
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let value = ((state >> 40) as f32 / 16_777_216.0 * 2.0 - 1.0) * 0.5;
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap();
    let looped = |start: f32, fade_s: f32| {
        let mut layer = LayerParams {
            loop_mode: LoopMode::Forward,
            start,
            loop_start: LOOP_START as f32 / last,
            loop_end: LOOP_END as f32 / last,
            loop_crossfade_s: fade_s,
            ..LayerParams::default()
        };
        layer.root_note = 60.0;
        layer
    };

    // Level. Repitch at unity from the region start reaches Loop end at output sample 36 000, so the
    // fade before the seam is output samples 31 200..36 000.
    let render = |source: &Sample, fade_s: f32| -> Vec<f32> {
        let mut params = Params {
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        params.layers[0] = looped(0.0, fade_s);
        let mut engine = Engine::new(RATE);
        engine.note_on(Note::midi(60, 1.0), [Some(source), None], &params);
        (0..LOOP_END + 1_000)
            .map(|_| engine.render([Some(source), None], &params).left)
            .collect()
    };
    let whole = LOOP_END - FADE..LOOP_END;
    let centre = LOOP_END - FADE / 2 - FADE / 8..LOOP_END - FADE / 2 + FADE / 8;
    for (name, source) in [
        ("a loop that closes (tone over whole cycles)", &tone),
        ("ends that do not agree (noise)", &noise),
    ] {
        let hard = render(source, 0.0);
        let faded = render(source, FADE as f32 / RATE);
        let db = |span: std::ops::Range<usize>| {
            20.0 * (rms(&faded[span.clone()]) / rms(&hard[span]).max(f64::MIN_POSITIVE)).log10()
        };
        println!(
            "loop crossfade level, {name}: {:+.2} dB over the fade's central quarter, {:+.2} dB over the whole {FADE}-frame fade, against the hard wrap",
            db(centre.clone()),
            db(whole.clone()),
        );
    }

    // Cost, where the seam is widest: every voice starts at Loop start, and a fade at its ceiling —
    // half the loop — makes the whole shortened loop fade, so every read goes through it.
    for reader in [Reader::Repitch, Reader::Stretch] {
        for (label, fade_s) in [("no fade", 0.0f32), ("the whole loop fading", 2.0)] {
            let mut params = Params::default();
            for layer in &mut params.layers {
                *layer = LayerParams {
                    reader,
                    ..looped(LOOP_START as f32 / last, fade_s)
                };
            }
            let mut engine = Engine::new(RATE);
            for note in 48..48 + MAX_VOICES as u8 {
                engine.note_on(Note::midi(note, 1.0), [Some(&tone), Some(&tone)], &params);
            }
            let rendered_frames = 240_000usize;
            let started = Instant::now();
            let mut checksum = 0.0f32;
            for _ in 0..rendered_frames {
                let value = engine.render([Some(&tone), Some(&tone)], &params);
                checksum += value.left + value.right;
            }
            black_box(checksum);
            let elapsed = started.elapsed().as_secs_f64();
            let audio_seconds = f64::from(rendered_frames as f32 / RATE);
            println!(
                "loop crossfade CPU probe, {reader:?} with {label}: {MAX_VOICES} voices × 2 layers, {:.3} s for {:.1} s audio ({:.1}% of one core realtime)",
                elapsed,
                audio_seconds,
                elapsed / audio_seconds * 100.0,
            );
        }
    }
}
