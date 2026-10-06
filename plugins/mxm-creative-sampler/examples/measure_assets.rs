//! Measures the provisional worst-case asset cap off the audio thread.
//! Run in release; source-file generation is deliberately excluded from the preparation timing.
//!
//! Files given after `--` are imported too, each into a fresh field, and timed: that is how an
//! import of any format the decoder reads is costed against the WAV figure above.
//!
//! ```text
//! cargo run -p mxm-creative-sampler --release --example measure_assets -- long.flac long.mp3
//! ```

use mxm_creative_sampler::asset::{AssetField, MAX_FRAMES_PER_LAYER};
use std::time::Instant;

fn main() {
    let path = std::env::temp_dir().join(format!(
        "mxm-creative-sampler-{}-asset-budget.wav",
        std::process::id()
    ));
    let samples: Vec<f32> = (0..MAX_FRAMES_PER_LAYER)
        .map(|frame| f32::from((((frame % 257) as i32 - 128) * 127) as i16) / 32_767.0)
        .collect();
    mxm_audio_file::write(
        &path,
        &samples,
        1,
        48_000,
        mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
    )
    .unwrap();
    drop(samples);

    let assets = AssetField::new();
    let started = Instant::now();
    assets.import_wav(0, &path).unwrap();
    let first = started.elapsed();
    let started = Instant::now();
    assets.import_wav(1, &path).unwrap();
    let second = started.elapsed();
    let started = Instant::now();
    let state = serde_json::to_vec(&assets.preset_state()).unwrap();
    let serialized = started.elapsed();
    let started = Instant::now();
    assets.clear_layer(0).unwrap();
    assets.clear_layer(1).unwrap();
    let cleared = started.elapsed();
    std::fs::remove_file(&path).unwrap();

    println!("frames/layer: {MAX_FRAMES_PER_LAYER}");
    println!("prepare A: {:.3} s", first.as_secs_f64());
    println!(
        "prepare B while A is resident: {:.3} s",
        second.as_secs_f64()
    );
    println!(
        "two-layer preset payload: {} bytes ({:.2} MiB), serialize {:.3} s",
        state.len(),
        state.len() as f64 / (1024.0 * 1024.0),
        serialized.as_secs_f64()
    );
    println!(
        "clear/reclaim both layers off audio: {:.3} s",
        cleared.as_secs_f64()
    );

    for file in std::env::args().skip(1) {
        let assets = AssetField::new();
        let started = Instant::now();
        let outcome = assets.import_wav(0, std::path::Path::new(&file));
        let elapsed = started.elapsed();
        // An import stages; until it commits, the bank still reads the starting source.
        assets.commit();
        let frames = assets.bank().read().samples[0]
            .as_ref()
            .map_or(0, |sample| sample.len());
        match outcome {
            Ok(()) => println!(
                "import {file}: {:.3} s, {frames} frames",
                elapsed.as_secs_f64()
            ),
            Err(error) => println!(
                "import {file}: refused after {:.3} s: {error}",
                elapsed.as_secs_f64()
            ),
        }
    }
}
