//! A file imported into a layer, and what the plugin decides when a source arrives.
//!
//! **Any format the collection's decoder reads** — WAV, AIFF, FLAC, ALAC, MP3, AAC in M4A and Ogg
//! Vorbis, judged by its content rather than its extension (mxm-kit's `crates/mxm-audio-file-decode`).
//! The instrument's policy is here: more than [`MAX_FRAMES_PER_LAYER`] frames is refused, the first
//! two channels are kept and mono is duplicated, and the audio is quantised once to canonical stereo
//! i16. Integer PCM arrives scaled by `2^(bits−1)`, the rule this import used when hound read it, so
//! a WAV imports to the same canonical state it did before.
//!
//! **The arrival** (`plans/plan-sampler-wav-loop-import.md` §1.2, §1.3) is every source-owned value a
//! new source writes before anyone hears it: the region, the loop, Root. The plugin's editor writes
//! [`Arrived::writes`] as host gestures before it publishes; [`crate::State::arrive`] writes the
//! same values into a state.

use std::io::{Cursor, Read, Seek};
use std::path::Path;

use mxm_audio_file_decode::{AtLimit, Container, Decoded, Error, Keep, Limits};
use mxm_creative_sampler_dsp::{LoopMode, Sample};

use crate::params::{LAYERS, known};
use crate::payload::{
    EncodedSample, MAX_FRAMES_PER_LAYER, canonical_frame, encode_pcm, quantise, reserve,
};
use crate::sound::LOOP_MODES;
use crate::wav_loop::{self, FileLoop};

/// A file's audio, canonical, and the loop it carries when it carries a usable one.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub sample: EncodedSample,
    pub file_loop: Option<FileLoop>,
}

/// What a staged source says about its own loop and pitch, for the source-owned defaults written
/// before anyone hears it (`plans/plan-sampler-wav-loop-import.md` §1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
    /// A generated source: built to loop over its whole buffer, with its root in its name.
    Generated,
    /// A dropped file, and the loop it carries when it carries a usable one.
    File(Option<FileLoop>),
}

impl Imported {
    /// What this import says about itself when it arrives.
    #[must_use]
    pub fn arrival(&self) -> Arrival {
        Arrival::File(self.file_loop)
    }
}

fn limits() -> Limits {
    Limits::new(MAX_FRAMES_PER_LAYER, AtLimit::Refuse, Keep::First(2))
}

fn refusal(error: Error) -> String {
    match error {
        Error::TooLong { .. } => format!(
            "too long for this small-sample instrument ({MAX_FRAMES_PER_LAYER} frames maximum)"
        ),
        other => other.to_string(),
    }
}

/// The file at `path`, imported. **A file loops when it says it does**: its `smpl` loop arrives with
/// it; a file with none is not forced to loop.
pub fn decode_file(path: &Path) -> Result<Imported, String> {
    let decoded = mxm_audio_file_decode::decode_file(path, &limits()).map_err(refusal)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sample.wav")
        .to_owned();
    finish(decoded, name, || {
        std::fs::File::open(path).ok().map(std::io::BufReader::new)
    })
}

/// A file's bytes, imported as [`decode_file`] imports the file: **for a program with no file
/// system**, a browser among them. `name` is the file's name, which the layer carries and whose
/// extension is the decoder's hint, as a path's is.
///
/// The decoder takes its own copy of the bytes, reserved fallibly.
pub fn decode_bytes(name: &str, bytes: &[u8]) -> Result<Imported, String> {
    let mut owned = Vec::new();
    reserve(&mut owned, bytes.len(), "import file")?;
    owned.extend_from_slice(bytes);
    let extension = Path::new(name).extension().and_then(|e| e.to_str());
    let decoded = mxm_audio_file_decode::decode(Box::new(Cursor::new(owned)), extension, &limits())
        .map_err(refusal)?;
    finish(decoded, name.to_owned(), || Some(Cursor::new(bytes)))
}

/// The decoded audio made canonical, with the loop `chunks` carries when the file is a WAV.
fn finish<R: Read + Seek>(
    decoded: Decoded,
    name: String,
    chunks: impl FnOnce() -> Option<R>,
) -> Result<Imported, String> {
    let container = decoded.container;
    let sample_rate = decoded.sample_rate;
    let channels = decoded.channels;
    let actual_frames = decoded.frames();
    // The decoder's own buffer is `mxm-audio-file-decode`'s to reserve. Everything this import sizes
    // from the file after it is reserved fallibly, and the samples are clamped where they lie.
    let mut interleaved = decoded.interleaved;
    let pcm = quantise(&mut interleaved, channels, actual_frames)?;
    // **Optional metadata adds no failure** (§1.2): the audio above is the import, and a chunk list
    // that cannot be read, or holds no usable loop, only means the file has none.
    // Only a RIFF WAVE carries a `smpl` chunk; any other container has no loop to read.
    let file_loop = (container == Container::Wav)
        .then(chunks)
        .flatten()
        .and_then(|mut reader| wav_loop::read_loop(&mut reader, actual_frames as u32))
        .map(|mut found| {
            // Measured only for a loop that repeats fast enough to be a pitch, so a phrase-length
            // loop never builds a copy of itself — and **on the canonical audio a layer plays**, not
            // the decoded floats (`canonical_frame`).
            let span = (found.end - found.start + 1) as usize;
            let rate = sample_rate as f32;
            if span as f32 <= rate / wav_loop::LOOP_PITCH_FLOOR_HZ {
                let cycle: Vec<[f32; 2]> = pcm
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .skip(found.start as usize)
                    .take(span)
                    .map(canonical_frame)
                    .collect();
                found.root = wav_loop::loop_pitch(&cycle, rate, found.unity).root;
            }
            found
        });
    let encoded = encode_pcm(&pcm)?;
    Ok(Imported {
        sample: EncodedSample {
            name,
            sample_rate,
            frames: actual_frames as u32,
            pcm: encoded,
        },
        file_loop,
    })
}

/// Every source-owned value an arrival writes, decided before any of it is written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrived {
    /// Normalised Loop start and Loop end, or `None` for the parameters' own whole-file defaults.
    pub loop_points: Option<(f32, f32)>,
    /// The loop mode the source asks for, or `None` to leave the layer's.
    pub loop_mode: Option<LoopMode>,
    /// The crossfade in seconds the source asks for, or `None` to leave the layer's.
    pub crossfade: Option<f32>,
    /// Root, as a MIDI note, fractional.
    pub root: f32,
}

/// Where Root lands when an arrival says nothing better: C4, middle of the keyboard, and the
/// convention Root itself displays, so an import is at least consistent.
///
/// Not either layer's parameter default — see [`Arrived::writes`].
pub const NEUTRAL_ROOT: f32 = 60.0;

/// What an arrival writes (`plans/plan-sampler-wav-loop-import.md` §1.2, §1.3).
///
/// **A generated source is built to loop**: every partial in [`crate::generate`] takes a whole
/// number of cycles so the buffer joins to itself. Publishing one into a layer whose loop is off
/// made the instrument contradict its own design — Repitch and Stretch played the buffer once and
/// stopped while Grain kept sounding, because it never reads the loop — which is how it was
/// reported. So it arrives Forward over the whole buffer.
///
/// **A file that carries a loop arrives looping at its own points**, in the mode its chunk names
/// (owner, 2026-09-14: *they were made for looping*). **A file without one keeps the layer's loop
/// mode and crossfade** — a drum hit that suddenly repeats is a worse defect than a waveform that
/// does not — and its loop points go to the whole file, as every import's do.
///
/// **Both looping arrivals come with no crossfade.** A loop made to close is only harmed by a fade,
/// which moves its return point off a whole cycle and fades against a cycle offset; the knob is
/// there for a loop that was not made to close.
///
/// **Root: the loop's own pitch, then the key in the name, then C4** (owner, 2026-09-14, D4). The
/// loop is measured when the file is decoded, from the canonical audio a layer plays, exact to the
/// cent, and only for a loop that repeats fast enough to be a pitch (`wav_loop::loop_pitch`). The
/// name gives a pitch class and the audio its octave ([`root_in_name`]). Neither, and Root lands on
/// [`NEUTRAL_ROOT`].
///
/// `sample` is the arriving layer as the DSP plays it, which the name's octave is heard in.
#[must_use]
pub fn resolve(arrival: Arrival, name: &str, sample: Option<&Sample>) -> Arrived {
    let named = || root_in_name(name, sample);
    match arrival {
        Arrival::Generated => Arrived {
            loop_points: None,
            loop_mode: Some(LoopMode::Forward),
            crossfade: Some(0.0),
            root: named().unwrap_or(NEUTRAL_ROOT),
        },
        Arrival::File(None) => Arrived {
            loop_points: None,
            loop_mode: None,
            crossfade: None,
            root: named().unwrap_or(NEUTRAL_ROOT),
        },
        Arrival::File(Some(found)) => {
            let last = found.frames.saturating_sub(1).max(1) as f32;
            Arrived {
                loop_points: Some((found.start as f32 / last, found.end as f32 / last)),
                loop_mode: Some(if found.alternate {
                    LoopMode::Alternate
                } else {
                    LoopMode::Forward
                }),
                crossfade: Some(0.0),
                root: found.root.or_else(named).unwrap_or(NEUTRAL_ROOT),
            }
        }
    }
}

impl Arrived {
    /// **Every write this arrival makes on `layer`, in order, each a parameter id and the
    /// normalised value it lands on**: the region, both loop points, Root, and the loop mode and
    /// crossfade where the arrival sets them. The plugin writes each as one complete host gesture.
    ///
    /// **Root does not go back to its parameter default, because A's default is the init saw's own
    /// pitch.** `aroot` starts at C3 so a fresh instance is in tune with the generated saw, and
    /// `broot` at C4 because layer B has no saw to be in tune with. That is right until a file is
    /// dropped: the saw is gone, C3 means nothing to the new audio, and the same file dropped on A
    /// and on B came out an octave apart.
    pub fn writes(&self, layer: usize) -> impl Iterator<Item = (&'static str, f32)> + use<> {
        let ids = LAYERS[layer];
        let default = |id: &'static str| (id, known(id).default_normalized());
        let (from, to) = self.loop_points.unwrap_or((
            known(ids.loop_start).default_normalized(),
            known(ids.loop_end).default_normalized(),
        ));
        [
            Some(default(ids.start)),
            Some(default(ids.end)),
            Some((ids.loop_start, from)),
            Some((ids.loop_end, to)),
            // Asked for by value rather than by a hand-written 0.5, so adding a fourth loop mode
            // cannot silently re-point this at the wrong one.
            self.loop_mode.map(|mode| {
                let choice = known(ids.loop_mode);
                (
                    ids.loop_mode,
                    choice.normalized(LOOP_MODES.index_of(mode) as f32),
                )
            }),
            self.crossfade.map(|seconds| {
                (
                    ids.loop_crossfade,
                    known(ids.loop_crossfade).normalized(seconds),
                )
            }),
            Some((ids.root, self.root / 127.0)),
        ]
        .into_iter()
        .flatten()
    }
}

/// Root from the key in the file's name, when the name carries one.
///
/// **A drop should arrive in tune when the file says how.** About one file in seven in a commercial
/// library ends its name with a key. A file's `smpl` chunk comes first when its loop is a pitch
/// ([`resolve`]); in that library the chunk's unity note is absent or set to zero — see
/// [`crate::naming`] for the survey.
///
/// The name gives a pitch class and the audio gives the octave, because a name almost never carries
/// one. That pairing is also what makes this better than either half: the class corrects the
/// detector's whole-note mistakes, and the detector keeps the answer in the register the sample
/// actually sits in.
///
/// A name with no key says nothing. **Nothing is guessed from the audio alone** — a detector's answer
/// is confident on a bass note and wrong on a chord, and a wrong Root retunes the whole keyboard.
/// That is what the plugin's Auto button is for: asked, not assumed. The one narrowing is a loop the
/// file carries, whose length states the pitch (`wav_loop::loop_pitch`).
#[must_use]
pub fn root_in_name(name: &str, sample: Option<&Sample>) -> Option<f32> {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    // **The name is consulted before the audio, so an unpitched sample is never analysed.** Plenty
    // of music is made from material that has no pitch at all, and running a detector over a drum
    // loop on every drop is work whose answer is thrown away — or worse, kept. Detection happens
    // only when a name has claimed a key and left out the octave, which is the one question the
    // audio is actually being asked.
    let tag = crate::naming::key_in_name(stem)?;
    let heard = match tag.octave {
        // A name that carries its octave is already a note; there is nothing to listen for.
        Some(_) => None,
        None => sample.and_then(|sample| mxm_creative_sampler_dsp::detect_root(sample, 0.0)),
    };
    Some(crate::naming::root_from_key(tag, heard))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A loop's pitch is heard in the audio a layer plays, not in the decoded floats** (code review
    /// round 2). A float loop just over the detector's level floor is pitched as decoded; quantised to
    /// 16 bits its peak falls under that floor, so an import that listened to the floats would
    /// retune the keyboard from audio the layer does not play. The unity note is unset, so the
    /// detector is the guide.
    #[test]
    fn a_loop_pitch_is_heard_in_the_audio_a_layer_plays() {
        use std::f32::consts::TAU;
        let dir = std::env::temp_dir().join(format!(
            "mxm-creative-sampler-canonical-pitch-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let cycle = 169u32;
        let tone =
            |amplitude: f32, n: u32| (TAU * (n % cycle) as f32 / cycle as f32).sin() * amplitude;
        let write = |name: &str, amplitude: f32| {
            let samples: Vec<f32> = (0..4_096u32).map(|n| tone(amplitude, n)).collect();
            let (mut bytes, _) =
                mxm_audio_file::encode(&samples, 1, 44_100, mxm_audio_file::Target::WavFloat32)
                    .unwrap();
            // One forward loop over the first cycle, unity note unset.
            let mut body = vec![0u8; 36];
            body[28..32].copy_from_slice(&1u32.to_le_bytes());
            for word in [0u32, 0, 0, cycle - 1, 0, 0] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            bytes.extend_from_slice(b"smpl");
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&body);
            let riff = (bytes.len() - 8) as u32;
            bytes[4..8].copy_from_slice(&riff.to_le_bytes());
            let path = dir.join(name);
            std::fs::write(&path, bytes).expect("a scratch WAV");
            path
        };

        // Loud: the control, that a float import measures a pitch at all.
        let loud = decode_file(&write("loud.wav", 0.5)).expect("a float WAV decodes");
        let root = loud
            .file_loop
            .and_then(|found| found.root)
            .expect("a loud single cycle is a pitch");
        assert!(
            (root - 59.955).abs() < 0.01,
            "the loud cycle's root is {root}"
        );

        // Quiet: pitched as floats, a step under the floor as played.
        let amplitude = 1.05e-4;
        let floats: Vec<[f32; 2]> = (0..cycle)
            .map(|n| [tone(amplitude, n), tone(amplitude, n)])
            .collect();
        assert!(
            wav_loop::loop_pitch(&floats, 44_100.0, None).root.is_some(),
            "the quiet cycle is not pitched even as floats, so this case proves nothing"
        );
        let quiet = decode_file(&write("quiet.wav", amplitude)).expect("a float WAV decodes");
        let found = quiet
            .file_loop
            .expect("the quiet file's loop is still read");
        assert_eq!(
            found.root, None,
            "a loop under the detector's floor as played set Root from its decoded floats"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A file's bytes import as the file does**: the same payload and the same loop, for a WAV
    /// with a loop and for one without.
    #[test]
    fn bytes_import_as_the_file_does() {
        let dir = std::env::temp_dir().join(format!(
            "mxm-creative-sampler-bytes-import-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let samples: Vec<f32> = (0..2_000u32)
            .map(|n| ((n * 37 % 400) as f32 / 200.0 - 1.0) * 0.5)
            .collect();
        let (plain, _) = mxm_audio_file::encode(
            &samples,
            1,
            48_000,
            mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
        )
        .unwrap();
        let mut looped = plain.clone();
        let mut body = vec![0u8; 36];
        body[12..16].copy_from_slice(&60u32.to_le_bytes());
        body[28..32].copy_from_slice(&1u32.to_le_bytes());
        for word in [0u32, 1, 100, 499, 0, 0] {
            body.extend_from_slice(&word.to_le_bytes());
        }
        looped.extend_from_slice(b"smpl");
        looped.extend_from_slice(&(body.len() as u32).to_le_bytes());
        looped.extend_from_slice(&body);
        let riff = (looped.len() - 8) as u32;
        looped[4..8].copy_from_slice(&riff.to_le_bytes());
        for (name, bytes) in [("plain_C.wav", plain), ("looped.wav", looped)] {
            let path = dir.join(name);
            std::fs::write(&path, &bytes).unwrap();
            let from_file = decode_file(&path).expect("the file imports");
            let from_bytes = decode_bytes(name, &bytes).expect("the bytes import");
            assert_eq!(from_bytes, from_file, "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
