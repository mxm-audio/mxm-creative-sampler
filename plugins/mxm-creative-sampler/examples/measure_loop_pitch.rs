//! How an import hears every looped WAV in a folder: the loop `wav_loop::read_loop` finds, how often
//! it repeats, the note `wav_loop::loop_pitch` hears in it played repeated, how many cycles that puts
//! in the loop, and the Root it would set. **The evidence behind `wav_loop::WHOLE_CYCLE_TOLERANCE`**
//! (`plans/plan-sampler-wav-loop-import.md`, D5).
//!
//! The files measured are the owner's own and are third-party audio, so they stay outside the
//! repository; point this at them:
//!
//! ```text
//! cargo run -p mxm-creative-sampler --release --example measure_loop_pitch -- <folder>
//! ```

use mxm_creative_sampler::wav_loop::{loop_pitch, read_loop};
use std::io::BufReader;
use std::path::PathBuf;

fn main() {
    let folder = std::env::args()
        .nth(1)
        .expect("usage: measure_loop_pitch <folder of WAV files>");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .expect("a readable folder")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        })
        .collect();
    paths.sort();
    println!(
        "{:<22} {:>13} {:>9} {:>6} {:>8} {:>8} {:>8}",
        "file", "loop", "repeat Hz", "unity", "guide", "cycles", "root"
    );
    let show = |value: Option<f32>| value.map_or_else(|| "-".to_owned(), |v| format!("{v:.3}"));
    for path in paths {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let limits = mxm_audio_file_decode::Limits::new(
            usize::MAX,
            mxm_audio_file_decode::AtLimit::Refuse,
            mxm_audio_file_decode::Keep::First(2),
        );
        let Ok(decoded) = mxm_audio_file_decode::decode_file(&path, &limits) else {
            println!("{name:<22} not audio the decoder reads");
            continue;
        };
        let channels = decoded.channels;
        let frames = decoded.frames() as u32;
        let Some(found) = std::fs::File::open(&path)
            .ok()
            .and_then(|file| read_loop(&mut BufReader::new(file), frames))
        else {
            println!("{name:<22} no usable loop");
            continue;
        };
        let values = &decoded.interleaved;
        let cycle: Vec<[f32; 2]> = values
            .chunks_exact(channels)
            .skip(found.start as usize)
            .take((found.end - found.start + 1) as usize)
            .map(|frame| [frame[0], if channels == 1 { frame[0] } else { frame[1] }])
            .collect();
        let pitch = loop_pitch(&cycle, decoded.sample_rate as f32, found.unity);
        // What the detector alone would count, for the comparison the unity note won on.
        let heard = loop_pitch(&cycle, decoded.sample_rate as f32, None);
        println!(
            "{name:<22} {:>6}..{:<6} {:>9.2} {:>6} {:>8} {:>8} {:>8}   heard alone {} -> {}",
            found.start,
            found.end,
            pitch.repeat_hz,
            show(found.unity),
            show(pitch.guide),
            show(pitch.cycles),
            show(pitch.root),
            show(heard.cycles),
            show(heard.root)
        );
    }
}
