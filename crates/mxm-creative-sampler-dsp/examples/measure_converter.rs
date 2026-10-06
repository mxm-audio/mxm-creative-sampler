//! Measures each converter stage against each reader, to find where a stage stops being audible.
//!
//! The owner reported the converter section inaudible. The suspicion this tests: `Rate` and
//! `Reconstruction` are applied inside `read`, so they run **per grain**, while quantisation was
//! deliberately moved out to the voice sum because a cloud of decorrelated grains averages a
//! per-grain nonlinearity away. If that reasoning is right for quantisation it is right for these
//! two, and the numbers below should show them collapsing between Repitch and Grain.

use mxm_creative_sampler_dsp::{
    Character, Engine, LayerParams, LoopMode, Note, Params, Reader, Sample,
};
use std::f32::consts::TAU;

const RATE: f32 = 48_000.0;
const FRAMES: usize = 32_768;
const SKIP: usize = 8_192;

/// The instrument's own init source: a band-limited saw, which is what the owner is listening to.
fn saw() -> Sample {
    let cycles = 142.0;
    let len = 52_105;
    Sample::new(
        (0..len)
            .map(|n| {
                let phase = n as f32 / len as f32 * cycles;
                let mut value = 0.0;
                let mut harmonic = 1.0;
                while harmonic * cycles * RATE / len as f32 <= 12_000.0 {
                    value += (TAU * phase * harmonic).sin() / harmonic;
                    harmonic += 1.0;
                }
                let value = value * 0.3;
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap()
}

fn render(source: &Sample, reader: Reader, character: Character) -> Vec<f32> {
    render_at(source, reader, character, 60)
}

fn render_at(source: &Sample, reader: Reader, character: Character, key: u8) -> Vec<f32> {
    let mut params = Params::default();
    params.layers[0] = LayerParams {
        reader,
        loop_mode: LoopMode::Forward,
        ..params.layers[0]
    };
    params.character = character;
    let mut engine = Engine::new(RATE);
    engine.note_on(Note::midi(key, 1.0), [Some(source), None], &params);
    (0..FRAMES)
        .map(|_| engine.render([Some(source), None], &params).left)
        .skip(SKIP)
        .collect()
}

/// A steady sine, for reading the converter's image frequencies off a spectrum.
fn sine(hz: f32) -> Sample {
    Sample::new(
        (0..48_000)
            .map(|n| {
                let value = 0.6 * (TAU * hz * n as f32 / RATE).sin();
                [value, value]
            })
            .collect(),
        RATE,
    )
    .unwrap()
}

/// Magnitude at one frequency, by Goertzel — cheaper than an FFT for a handful of probes.
fn goertzel(v: &[f32], hz: f32, rate: f32) -> f32 {
    let k = TAU * hz / rate;
    let coeff = 2.0 * k.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &x in v {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / v.len() as f32
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
}

/// How much of `b` is not explained by scaling `a`: the part a level trim cannot recover.
fn residual(a: &[f32], b: &[f32]) -> f32 {
    let denom = a.iter().map(|x| x * x).sum::<f32>().max(1e-12);
    let gain = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / denom;
    let diff: Vec<f32> = a.iter().zip(b).map(|(x, y)| y - gain * x).collect();
    rms(&diff) / rms(b).max(1e-12)
}

fn main() {
    let source = saw();
    let clean = Character {
        rate: 0.0,
        converter: 0.0,
        reconstruction: 0.0,
        input: 0.0,
        ..Character::default()
    };

    let stages: [(&str, Character); 4] = [
        ("rate", Character { rate: 1.0, ..clean }),
        (
            "converter",
            Character {
                converter: 1.0,
                ..clean
            },
        ),
        (
            "reconstruction",
            Character {
                reconstruction: 1.0,
                ..clean
            },
        ),
        (
            "input",
            Character {
                input: 1.0,
                ..clean
            },
        ),
    ];

    println!("residual = share of the render a level trim cannot undo (0 = inaudible change)\n");
    println!(
        "{:<16} {:>12} {:>12} {:>12}",
        "stage at 100%", "Repitch", "Stretch", "Grain"
    );
    for (name, character) in stages {
        let mut row = format!("{name:<16}");
        for reader in [Reader::Repitch, Reader::Stretch, Reader::Grain] {
            let base = render(&source, reader, clean);
            let moved = render(&source, reader, character);
            row += &format!("{:>12.4}", residual(&base, &moved));
        }
        println!("{row}");
    }

    // **Unity rate is the case the owner tests on**, because the init patch's root is C3 and the
    // note under the mouse is C3. At unity the read position lands on whole frames, so there is no
    // fraction for the reconstruction stage to shape and it is a no-op — worth naming, because it
    // means the control is dead on exactly the note a player reaches for first.
    println!("\nresidual by key, Repitch (root is 60, so 60 plays at unity rate):\n");
    println!(
        "{:<16} {:>12} {:>12} {:>12}",
        "stage at 100%", "key 60", "key 65", "key 72"
    );
    for (name, character) in stages {
        let mut row = format!("{name:<16}");
        for key in [60u8, 65, 72] {
            let base = render_at(&source, Reader::Repitch, clean, key);
            let moved = render_at(&source, Reader::Repitch, character, key);
            row += &format!("{:>12.4}", residual(&base, &moved));
        }
        println!("{row}");
    }

    // **Does the virtual converter varispeed?** The hardware this borrows from made pitch by
    // changing a per-voice DAC's clock — the Fairlight, the Emulator II, the Synclavier — so the
    // converter's own rate moved with the note, and its images moved with it. `grid` is counted in
    // *source frames*, so the question is whether that already falls out of the arithmetic.
    //
    // Read it off the spectrum. A 400 Hz sine held at full hold, grid 32, played at the root and an
    // octave below. At the root the converter runs at 48000/32 = 1500 Hz and the lower image sits
    // at 1500 − 400 = 1100 Hz. An octave down the fundamental is 200 Hz, and the image is at
    // 750 − 200 = 550 Hz if the clock tracks the note, or 1500 − 200 = 1300 Hz if it does not.
    println!("\ndoes the converter's clock follow the note? (400 Hz sine, full hold, grid 32)\n");
    let tone = sine(400.0);
    let held = Character {
        rate: 1.0,
        reconstruction: 1.0,
        ..clean
    };
    let root = render_at(&tone, Reader::Repitch, held, 60);
    let octave_down = render_at(&tone, Reader::Repitch, held, 48);
    println!(
        "  at the root, image expected 1100 Hz:  {:.5}   (1300 Hz: {:.5})",
        goertzel(&root, 1100.0, RATE),
        goertzel(&root, 1300.0, RATE)
    );
    println!(
        "  an octave down, varispeed says 550 Hz: {:.5}   fixed clock says 1300 Hz: {:.5}",
        goertzel(&octave_down, 550.0, RATE),
        goertzel(&octave_down, 1300.0, RATE)
    );

    // **A patch has to sound the same at every host rate.** The converter's grid is counted in
    // source frames, so its rate works out to `pitch_ratio × source_rate / grid` and the host rate
    // should cancel. Render the same patch at 44.1 and at 96 kHz and read the image off each: if
    // the host rate cancels, both land on the same hertz.
    // The **same** 44.1 kHz sample throughout — varying the source along with the host would be
    // testing nothing. Converter rate is then 44100/32 = 1378 Hz at the root whatever the host is,
    // so the image belongs at 1378 − 400 = 978 Hz in all three rows.
    println!(
        "\nis the converter independent of the host rate? (one 44.1 kHz sample, three hosts)\n"
    );
    let tone = Sample::new(
        (0..44_100)
            .map(|n| {
                let v = 0.6 * (TAU * 400.0 * n as f32 / 44_100.0).sin();
                [v, v]
            })
            .collect(),
        44_100.0,
    )
    .unwrap();
    for host in [44_100.0f32, 48_000.0, 96_000.0] {
        let mut params = Params::default();
        params.layers[0] = LayerParams {
            reader: Reader::Repitch,
            loop_mode: LoopMode::Forward,
            ..params.layers[0]
        };
        params.character = held;
        let mut engine = Engine::new(host);
        engine.note_on(Note::midi(60, 1.0), [Some(&tone), None], &params);
        let out: Vec<f32> = (0..(host as usize / 2))
            .map(|_| engine.render([Some(&tone), None], &params).left)
            .skip(8_192)
            .collect();
        println!(
            "  host {:>6.0} Hz   image at 978 Hz: {:.5}   fundamental 400 Hz: {:.5}",
            host,
            goertzel(&out, 978.0, host),
            goertzel(&out, 400.0, host)
        );
    }

    // **Grain shape is a tone control, so it has to change the tone and not the level.** The
    // moments follow the shape, so the normalisation should hold the RMS roughly flat while the
    // residual — the part no level trim explains — stays large.
    println!("\ngrain shape: does it change the tone without changing the level?\n");
    let cloudy = LayerParams {
        reader: Reader::Grain,
        loop_mode: LoopMode::Forward,
        grain_position_variation: 0.6,
        grain_density_hz: 40.0,
        ..LayerParams::default()
    };
    let at_shape = |shape: f32| {
        let mut params = Params::default();
        params.layers[0] = LayerParams {
            grain_shape: shape,
            ..cloudy
        };
        let mut engine = Engine::new(RATE);
        engine.note_on(Note::midi(60, 1.0), [Some(&source), None], &params);
        (0..FRAMES)
            .map(|_| engine.render([Some(&source), None], &params).left)
            .skip(SKIP)
            .collect::<Vec<f32>>()
    };
    let hann_end = at_shape(1.0);
    println!(
        "{:<12} {:>10} {:>14} {:>10}",
        "shape", "rms", "vs Smooth", "residual"
    );
    for (name, shape) in [
        ("Hard", 0.0),
        ("Triangle", 0.5),
        ("75%", 0.75),
        ("Smooth", 1.0),
    ] {
        let rendered = at_shape(shape);
        println!(
            "{:<12} {:>10.5} {:>11.2} dB {:>10.4}",
            name,
            rms(&rendered),
            20.0 * (rms(&rendered) / rms(&hann_end).max(1e-9))
                .max(1e-9)
                .log10(),
            residual(&hann_end, &rendered)
        );
    }

    println!("\nthe converter's taper — bit depth across the knob:");
    for step in 0..=10 {
        let c = step as f32 / 10.0;
        let bits = mxm_creative_sampler_dsp::character_bits(c);
        println!("  {:>3.0}%  {:>4.0} bits", c * 100.0, bits);
    }

    println!("\nrate's taper — virtual grid across the knob:");
    for step in 0..=10 {
        let r = step as f32 / 10.0;
        let grid = mxm_creative_sampler_dsp::character_rate_divisor(r);
        println!(
            "  {:>3.0}%  grid {:>6.2} frames  ({:>7.0} Hz effective)",
            r * 100.0,
            grid,
            RATE / grid
        );
    }
}
