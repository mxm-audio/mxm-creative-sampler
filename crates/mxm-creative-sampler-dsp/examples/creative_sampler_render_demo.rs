//! Renders the same project-authored chirped tone through Repitch, Stretch and Grain.

use mxm_creative_sampler_dsp::{Engine, Note, Params, Reader, Sample};
use std::f32::consts::TAU;

fn main() -> std::io::Result<()> {
    let rate = 48_000u32;
    let source = Sample::new(
        (0..rate as usize)
            .map(|frame| {
                let t = frame as f32 / rate as f32;
                let phase = TAU * (110.0 * t + 990.0 * t * t * 0.5);
                let value = phase.sin() * (1.0 - t).sqrt() * 0.65;
                [value, value]
            })
            .collect(),
        rate as f32,
    )
    .expect("the generated source is finite");
    let mut rendered = Vec::with_capacity(rate as usize * 6);
    for reader in [Reader::Repitch, Reader::Stretch, Reader::Grain] {
        let mut params = Params::default();
        params.layers[0].reader = reader;
        params.layers[0].loop_mode = mxm_creative_sampler_dsp::LoopMode::Forward;
        params.layers[0].grain_position_variation = 0.55;
        params.layers[0].grain_onset_timing = 0.55;
        let mut engine = Engine::new(rate as f32);
        engine.note_on(Note::midi(67, 0.9), [Some(&source), None], &params);
        for frame in 0..rate * 2 {
            if frame == rate + rate / 2 {
                engine.note_off(None, 0, 67);
            }
            let value = engine.render([Some(&source), None], &params);
            rendered.push(value.left);
            rendered.push(value.right);
        }
    }
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/mxm-creative-sampler-demo.wav");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_wav(&output, rate, &rendered)?;
    println!("wrote {}", output.display());
    Ok(())
}

/// Writes the demo through `mxm-measure`'s encoder.
///
/// **The eighth hand-written WAV writer.** Two censuses missed it: the first searched for the shared
/// `examples/common/wav.rs` module, the second for the `write_demo` helper name. Searching for
/// `b"RIFF"` is what finds them all, and it is what the plan records now. This one already rounded
/// rather than truncating and applied no headroom, so nothing about the output moves.
fn write_wav(path: &std::path::Path, sample_rate: u32, interleaved: &[f32]) -> std::io::Result<()> {
    mxm_audio_file::write(
        path,
        interleaved,
        2,
        sample_rate,
        mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
    )
    .map(|_| ())
    .map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This demo's own writer, the eighth of nine the census eventually found.
    ///
    /// The shared encoder is proved in `mxm-measure` against fixed header and payload bytes. What
    /// that cannot see is *this* file later writing the wrong channel count or rate, so the
    /// expectations here are literals — the facts about this instrument.
    #[test]
    fn the_demo_writer_produces_a_playable_file() {
        let frames = 256;
        let interleaved: Vec<f32> = (0..frames * 2)
            .map(|i| {
                let t = i as f32 / 48_000 as f32;
                0.8 * (std::f32::consts::TAU * 220.0 * t).sin()
            })
            .collect();

        let mut path = std::env::temp_dir();
        path.push(format!(
            "creative-sampler-render-writer-{}.wav",
            std::process::id()
        ));
        write_wav(&path, 48_000, &interleaved).expect("the demo is written");

        let read = mxm_audio_file_decode::decode_file(
            &path,
            &mxm_audio_file_decode::Limits::new(
                usize::MAX,
                mxm_audio_file_decode::AtLimit::Refuse,
                mxm_audio_file_decode::Keep::AllUpTo(2),
            ),
        )
        .expect("the demo file parses");
        assert_eq!(read.channels, 2, "the demo wrote the wrong channel count");
        assert_eq!(
            read.sample_rate, 48_000,
            "the demo wrote the wrong sample rate"
        );
        assert_eq!(read.frames(), frames, "the demo dropped or invented frames");

        // This writer applies no headroom law — it hands the samples to the encoder, which clamps —
        // so a 0.8 source arrives at 0.8 rather than scaled.
        let peak = mxm_measure::level::peak(&read.interleaved).expect("a finite file");
        assert!(
            (peak - 0.8).abs() < 0.01,
            "the writer changed the level: peak {peak}"
        );

        std::fs::remove_file(&path).ok();
    }
}
