//! Scores [`detect_root`] against a folder of samples whose file names carry the key.
//!
//! **Sample-library file names are the only ground truth available at this scale.** They are not
//! perfect — a pack's key tag is whatever its maker typed, and it names a pitch class with no
//! octave — but ten of them agreeing or disagreeing with the detector says far more than one
//! listen does, and disagreement is what points at the law to change.
//!
//! Run it against a folder:
//!
//! ```text
//! cargo run -p mxm-creative-sampler --example measure_root --release -- "<folder>"
//! ```

use mxm_creative_sampler_dsp::Sample;
use std::path::Path;

/// Reads any file the collection's decoder opens into the canonical stereo form the instrument uses.
fn read(path: &Path) -> Result<Sample, String> {
    let decoded = mxm_audio_file_decode::decode_file(
        path,
        &mxm_audio_file_decode::Limits::new(
            usize::MAX,
            mxm_audio_file_decode::AtLimit::Refuse,
            mxm_audio_file_decode::Keep::First(2),
        ),
    )
    .map_err(|e| e.to_string())?;
    let channels = decoded.channels;
    let sample_rate = decoded.sample_rate;
    let values = decoded.interleaved;
    let frames: Vec<[f32; 2]> = values
        .chunks(channels)
        .map(|frame| {
            let left = frame[0];
            let right = if channels > 1 { frame[1] } else { left };
            [left, right]
        })
        .collect();
    Sample::new(frames, sample_rate as f32).map_err(|e| format!("{e:?}"))
}

const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn note_name(note: f32) -> String {
    let n = note.round().clamp(0.0, 127.0) as i32;
    format!("{}{}", NAMES[(n % 12) as usize], n / 12 - 1)
}

/// The pitch class a pack's file name claims, read off the last underscore-separated token.
///
/// `OS_VV2_electric_bass_guitar_drop_D#.wav` claims D#. Tokens that are not a note are not a claim,
/// and the file is scored as unlabelled rather than as a failure.
fn claimed(stem: &str) -> Option<usize> {
    let token = stem.rsplit('_').next()?;
    // Packs write the key as a pitch class with an optional accidental and an optional mode:
    // `F`, `D#`, `Cm`, `Amin`, `Ebmaj`. The mode is not a pitch and is dropped.
    let token = token
        .trim()
        .trim_end_matches("maj")
        .trim_end_matches("min")
        .trim_end_matches('m')
        .trim_end_matches("MAJ")
        .trim_end_matches("MIN");
    let mut chars = token.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let accidental = chars.next();
    if chars.next().is_some() {
        return None;
    }
    let base = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let offset = match accidental {
        None => 0,
        Some('#') => 1,
        Some('b') => -1,
        Some(_) => return None,
    };
    Some(((base + offset) as usize + 12) % 12)
}

/// Prints what every window of one file heard, so a disagreement can be read rather than guessed
/// at: a loop that starts on the sixth is a different problem from one whose first note is missed.
fn trace(path: &Path) {
    let sample = match read(path) {
        Ok(sample) => sample,
        Err(why) => return println!("{why}"),
    };
    let rate = sample.sample_rate();
    let frames = sample.len() as f32;
    println!(
        "{} — {:.2} s at {:.0} Hz",
        path.display(),
        frames / rate,
        rate
    );
    // Walk the region start forward, so each reading begins a little later in the file. This is the
    // same `from` the Auto button passes, so it reports what the shipped detector would say if the
    // region started there.
    for step in 0..24 {
        let from = step as f32 * 0.02;
        if from >= 1.0 {
            break;
        }
        let at = from * frames / rate;
        match mxm_creative_sampler_dsp::detect_root(&sample, from) {
            Some(note) => println!("  from {at:>5.2} s   {:>5}  ({note:.2})", note_name(note)),
            None => println!("  from {at:>5.2} s       -"),
        }
    }
}

fn main() {
    let folder = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: measure_root <folder>");
        std::process::exit(2)
    });

    let target = Path::new(&folder);
    if target.is_file() {
        return trace(target);
    }

    let mut entries: Vec<_> = std::fs::read_dir(&folder)
        .unwrap_or_else(|e| panic!("cannot read {folder}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        })
        .collect();
    entries.sort();

    let (mut right, mut wrong, mut declined, mut unlabelled) = (0, 0, 0, 0);
    println!(
        "{:<48} {:>9} {:>10} {:>8}  verdict",
        "file", "claimed", "detected", "cents"
    );
    for path in &entries {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let want = claimed(stem);
        let sample = match read(path) {
            Ok(sample) => sample,
            Err(why) => {
                println!("{:<48} {why}", stem);
                continue;
            }
        };
        let found = mxm_creative_sampler_dsp::detect_root(&sample, 0.0);

        let claim = want.map(|c| NAMES[c].to_owned()).unwrap_or("-".to_owned());
        match (want, found) {
            (_, None) => {
                declined += 1;
                println!("{stem:<48} {claim:>9} {:>10} {:>8}  DECLINED", "-", "-");
            }
            (None, Some(note)) => {
                unlabelled += 1;
                println!(
                    "{stem:<48} {claim:>9} {:>10} {:>8}  (no claim)",
                    note_name(note),
                    ""
                );
            }
            (Some(want), Some(note)) => {
                // What the instrument would actually set on a drop: the name's pitch class, placed
                // in the octave the detector heard. Reported separately from the detector's own
                // verdict, so the two halves of the rule can be judged apart.
                if let Some(settled) = mxm_creative_sampler::naming::key_in_name(stem)
                    .map(|tag| mxm_creative_sampler::naming::root_from_key(tag, Some(note)))
                    && (settled - note.round()).abs() > 0.5
                {
                    let moved = (settled - note.round()).abs();
                    println!(
                        "{stem:<48} {claim:>9} {:>10} {:>8}  name corrects to {} ({moved:.0} st)",
                        note_name(note),
                        "",
                        note_name(settled)
                    );
                    right += 1;
                    continue;
                }
                let got = (note.round() as i32).rem_euclid(12) as usize;
                // Distance from the nearest whole semitone, so a detector that is right about the
                // note but drifting can be told from one that is on a different note entirely.
                let cents = (note - note.round()) * 100.0;
                if got == want {
                    right += 1;
                    println!(
                        "{stem:<48} {claim:>9} {:>10} {cents:>+8.0}  ok",
                        note_name(note)
                    );
                } else {
                    wrong += 1;
                    // Being an exact octave out is the classic YIN failure and worth naming.
                    let octave = if got == want { "" } else { " " };
                    println!(
                        "{stem:<48} {claim:>9} {:>10} {cents:>+8.0}  WRONG{octave}",
                        note_name(note)
                    );
                }
            }
        }
    }

    let labelled = right + wrong + declined;
    println!(
        "\n{right} right, {wrong} wrong, {declined} declined, of {labelled} labelled \
         ({unlabelled} unlabelled)"
    );
}
