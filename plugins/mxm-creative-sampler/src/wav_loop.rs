//! What a WAV says about its own loop, and the pitch that loop plays at.
//!
//! **Read beside the decoder, not through it.** `mxm-audio-file-decode` returns audio and no chunk, so
//! the RIFF chunk list is walked separately for a `smpl` chunk — header by header, seeking past
//! everything else, so the audio is never read twice. `plans/plan-sampler-wav-loop-import.md` §1.2
//! and §1.3.

use mxm_creative_sampler_dsp::{Sample, detect_root};
use std::io::{Read, Seek, SeekFrom};

/// The loop a WAV carries, in its own frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileLoop {
    /// The loop's first frame.
    pub start: u32,
    /// **The loop's last frame, inclusive**, as the chunk writes it and as Loop end means it.
    pub end: u32,
    /// The chunk's type 1, back and forth, rather than type 0, forward.
    pub alternate: bool,
    /// The file's frame count, so the points can be normalised without the audio.
    pub frames: u32,
    /// The note the file says it plays at unshifted — the chunk's unity note plus its pitch fraction —
    /// or `None` where the chunk leaves both at zero, which is unset rather than C-1: the survey behind
    /// [`crate::naming`] found every one of seventeen chunks in the owner's wider library that way.
    pub unity: Option<f32>,
    /// The pitch the loop plays at, as a MIDI note, when it is one (§1.3); `None` otherwise.
    pub root: Option<f32>,
}

/// The largest `smpl` chunk read. One this size already holds thousands of loops; a larger one is not
/// a sampler chunk worth trusting, and reading it would be an allocation a file chose.
const MAX_SMPL_BYTES: u32 = 64 * 1024;

/// How many chunks are walked before giving up. A WAV has a handful; a list that does not end within
/// this is not one.
const MAX_CHUNKS: usize = 1_024;

/// The first usable loop in a RIFF WAVE file's `smpl` chunk, or `None`.
///
/// **Never a reason to fail an import.** A file the decoder read imports its audio whatever its chunk
/// list holds, so every way this can go wrong — no `smpl` chunk, a truncated or oversized one, zero
/// loops, a loop that starts after it ends or ends past the last frame, a type this instrument has no
/// mode for — is simply no loop. The rules for a usable loop (§1.2):
///
/// - the **first** loop, and a second is ignored;
/// - type 0 is Forward and type 1 Alternate; **type 2, backward, is not usable** — there is no
///   backward loop, and Reverse would reverse the attack too;
/// - its points are whole frames, and the loop's `dwFraction` is ignored: the standard does not say
///   which point it refines, and nothing in the owner's library sets it;
/// - a finite play count still loops for as long as a note is held, because the instrument cannot
///   count passes.
pub fn read_loop<R: Read + Seek>(reader: &mut R, frames: u32) -> Option<FileLoop> {
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0u8; 12];
    reader.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    let declared = 8 + u64::from(u32::from_le_bytes(header[4..8].try_into().ok()?));
    let file_end = reader.seek(SeekFrom::End(0)).ok()?;
    let end = declared.min(file_end);
    let mut offset = 12u64;
    for _ in 0..MAX_CHUNKS {
        if offset + 8 > end {
            return None;
        }
        reader.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk = [0u8; 8];
        reader.read_exact(&mut chunk).ok()?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().ok()?);
        let body = offset + 8;
        if &chunk[0..4] == b"smpl" {
            if size > MAX_SMPL_BYTES || body + u64::from(size) > end {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            reader.read_exact(&mut data).ok()?;
            return first_loop(&data, frames);
        }
        // RIFF pads an odd-sized chunk to an even boundary.
        offset = body + u64::from(size) + u64::from(size & 1);
    }
    None
}

/// The first loop in a `smpl` chunk's body, if it is usable. The body is a 36-byte header — the unity
/// note at byte 12, its pitch fraction at 16, the loop count at 28 — followed by 24 bytes a loop: cue
/// id, type, start, end, fraction, play count.
fn first_loop(data: &[u8], frames: u32) -> Option<FileLoop> {
    let word = |at: usize| {
        data.get(at..at + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    if word(28)? == 0 {
        return None;
    }
    let alternate = match word(40)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let (start, end) = (word(44)?, word(48)?);
    // Back and forth over a single frame is that frame, which Forward plays exactly; Alternate would
    // turn a step either side of it and read its neighbours.
    let alternate = alternate && start != end;
    let (note, fraction) = (word(12)?, word(16)?);
    let unity = ((note != 0 || fraction != 0) && note <= 127)
        .then(|| note as f32 + (f64::from(fraction) / 4_294_967_296.0) as f32);
    (start <= end && end < frames).then_some(FileLoop {
        start,
        end,
        alternate,
        frames,
        unity,
        root: None,
    })
}

/// The slowest a loop can repeat and still be a pitch rather than a phrase. **The owner's rule**
/// (2026-09-14): *a drum loop might also have a loop but that is not pitch as it lasts longer than
/// audible frequencies.*
pub const LOOP_PITCH_FLOOR_HZ: f32 = 20.0;

/// How far from a whole number of cycles a loop's count may land and still be believed (D5).
///
/// **Measured on the owner's 91 JD-800 loops** with `examples/measure_loop_pitch.rs`: every loop that
/// holds whole cycles of its unity note counts within 0.05 of a whole number, and the three that do not
/// — `069 Strat Atk`, `050 AgogoBells` and `035 Fine Wine` — land 0.16 or more away and are refused. The
/// figures are in the plugin DOX.
pub const WHOLE_CYCLE_TOLERANCE: f32 = 0.1;

/// How much of the loop, repeated, the detector listens to. Several of its windows at the lowest pitch
/// it knows, with the attack slice it skips, and short enough to cost nothing beside a decode.
const LOOP_LISTEN_S: f32 = 1.0;

/// What measuring a loop's pitch found, step by step, for the measurement example as much as for the
/// import.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopPitch {
    /// How often the loop repeats, in Hz.
    pub repeat_hz: f32,
    /// The note the count was taken from: the file's unity note, or what the detector heard.
    pub guide: Option<f32>,
    /// Whether the guide was the detector's, because the file left its unity note unset.
    pub heard: bool,
    /// How many cycles the guide puts in the loop, `r`.
    pub cycles: Option<f32>,
    /// The note the loop plays at, when all of the above agree.
    pub root: Option<f32>,
}

/// The pitch a loop plays at, from the loop itself (§1.3).
///
/// **The loop gives the cents; the file's unity note gives the octave.** A loop may hold several
/// cycles, so its length alone puts a two-cycle loop an octave low. The count comes from a guide note:
///
/// 1. the guide is the file's unity note — or, **only where the file leaves it unset**, the detector's
///    note for the loop heard repeated, as it plays;
/// 2. the loop holds `r = f × P / rate` cycles of the guide's frequency `f`, and `n = round(r)` is
///    believed only when `n ≥ 1` and `r` is within [`WHOLE_CYCLE_TOLERANCE`] of it;
/// 3. the root is the note of `n × rate / P` Hz, exact to the cent.
///
/// **Why the unity note, where the plan first had the detector** (revision 7): on the owner's JD-800
/// set the detector, listening to a loop repeated, went wrong on 16 of the 71 loops fast enough to
/// be a pitch. Where a loop's cycles differ slightly it locks onto the whole loop — `029 Can Wave 1`
/// came out two octaves low and `069 Strat Atk` over two — and on others it caught a harmonic. Every
/// one of those files names its unity note, and counting against it puts every loop that holds whole
/// cycles on its own pitch and refuses the three that do not.
///
/// Nothing is set when the loop repeats slower than [`LOOP_PITCH_FLOOR_HZ`], when there is no guide —
/// no unity note and nothing the detector is confident of: a drum, noise, a chord — or when the count
/// is not a whole number.
pub fn loop_pitch(cycle: &[[f32; 2]], rate: f32, unity: Option<f32>) -> LoopPitch {
    let span = cycle.len() as f32;
    let mut found = LoopPitch {
        repeat_hz: if span > 0.0 { rate / span } else { 0.0 },
        guide: None,
        heard: false,
        cycles: None,
        root: None,
    };
    if cycle.is_empty() || found.repeat_hz < LOOP_PITCH_FLOOR_HZ {
        return found;
    }
    let guide = match unity {
        Some(note) => note,
        None => {
            let length = ((rate * LOOP_LISTEN_S) as usize).max(cycle.len());
            let repeated: Vec<[f32; 2]> = cycle.iter().copied().cycle().take(length).collect();
            let Some(note) = Sample::new(repeated, rate)
                .ok()
                .and_then(|sample| detect_root(&sample, 0.0))
            else {
                return found;
            };
            found.heard = true;
            note
        }
    };
    found.guide = Some(guide);
    let hz = 440.0 * ((guide - 69.0) / 12.0).exp2();
    let r = hz * span / rate;
    found.cycles = Some(r);
    let n = r.round();
    if n >= 1.0 && (r - n).abs() <= WHOLE_CYCLE_TOLERANCE {
        found.root = Some(69.0 + 12.0 * (n * rate / (span * 440.0)).log2());
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;
    use std::io::Cursor;

    /// A mono 16-bit WAV of `frames` frames, as the collection's encoder writes it.
    fn wav(frames: u32) -> Vec<u8> {
        let samples: Vec<f32> = (0..frames)
            .map(|n| f32::from((n as i16).wrapping_mul(7)) / 32_767.0)
            .collect();
        mxm_audio_file::encode(
            &samples,
            1,
            44_100,
            mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
        )
        .unwrap()
        .0
    }

    /// `bytes` with a chunk appended and the RIFF size brought up to date.
    fn with_chunk(mut bytes: Vec<u8>, id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        bytes.extend_from_slice(id);
        bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(body);
        if body.len() % 2 == 1 {
            bytes.push(0);
        }
        let riff = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        bytes
    }

    /// A `smpl` body with unity note 60 and the given loops, each `(type, start, end, fraction, play
    /// count)`.
    fn smpl(loops: &[(u32, u32, u32, u32, u32)]) -> Vec<u8> {
        let mut body = vec![0u8; 36];
        body[12..16].copy_from_slice(&60u32.to_le_bytes());
        body[28..32].copy_from_slice(&(loops.len() as u32).to_le_bytes());
        for (index, &(kind, start, end, fraction, count)) in loops.iter().enumerate() {
            for word in [index as u32, kind, start, end, fraction, count] {
                body.extend_from_slice(&word.to_le_bytes());
            }
        }
        body
    }

    fn read(bytes: &[u8], frames: u32) -> Option<FileLoop> {
        read_loop(&mut Cursor::new(bytes), frames)
    }

    #[test]
    fn a_forward_loop_is_read_on_its_frames() {
        let bytes = with_chunk(wav(169), b"smpl", &smpl(&[(0, 0, 168, 0, 0)]));
        assert_eq!(
            read(&bytes, 169),
            Some(FileLoop {
                start: 0,
                end: 168,
                alternate: false,
                frames: 169,
                unity: Some(60.0),
                root: None,
            })
        );
        // **One frame is a loop** — the half-open span of a frame naming itself — and back and forth
        // over one frame is that frame, forward.
        for kind in [0, 1] {
            let bytes = with_chunk(wav(169), b"smpl", &smpl(&[(kind, 42, 42, 0, 0)]));
            let found = read(&bytes, 169).expect("a one-frame loop is a loop");
            assert_eq!((found.start, found.end, found.alternate), (42, 42, false));
        }
    }

    /// **A unity note of zero is unset**, and a pitch fraction is cents above it.
    #[test]
    fn the_unity_note_is_read_as_the_file_means_it() {
        let mut unset = smpl(&[(0, 0, 168, 0, 0)]);
        unset[12..16].copy_from_slice(&0u32.to_le_bytes());
        let found = read(&with_chunk(wav(169), b"smpl", &unset), 169).expect("a loop");
        assert_eq!(found.unity, None, "a zero unity note was read as C-1");

        let mut sharp = smpl(&[(0, 0, 168, 0, 0)]);
        sharp[16..20].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        let found = read(&with_chunk(wav(169), b"smpl", &sharp), 169).expect("a loop");
        assert_eq!(
            found.unity,
            Some(60.5),
            "the pitch fraction was not half a note"
        );
    }

    /// **Every way a chunk can be unusable is no loop, and none is a failure.**
    #[test]
    fn an_unusable_loop_is_no_loop() {
        let base = wav(1_000);
        let unusable = [
            ("no smpl chunk", base.clone()),
            ("zero loops", with_chunk(base.clone(), b"smpl", &smpl(&[]))),
            (
                "start after end",
                with_chunk(base.clone(), b"smpl", &smpl(&[(0, 600, 500, 0, 0)])),
            ),
            (
                "end past the last frame",
                with_chunk(base.clone(), b"smpl", &smpl(&[(0, 100, 1_000, 0, 0)])),
            ),
            (
                "a backward loop",
                with_chunk(base.clone(), b"smpl", &smpl(&[(2, 100, 500, 0, 0)])),
            ),
            (
                "a header with no loop in it",
                with_chunk(base.clone(), b"smpl", &smpl(&[])[..30]),
            ),
        ];
        for (what, bytes) in unusable {
            assert_eq!(read(&bytes, 1_000), None, "{what} was read as a loop");
        }

        // Truncated: the chunk claims more than the file holds.
        let mut truncated = with_chunk(base.clone(), b"smpl", &smpl(&[(0, 100, 500, 0, 0)]));
        truncated.truncate(truncated.len() - 10);
        assert_eq!(read(&truncated, 1_000), None, "a truncated chunk was read");

        // Not a WAV at all.
        assert_eq!(read(b"RIFX\0\0\0\0WAVE", 1_000), None);
        assert_eq!(read(b"", 1_000), None);
    }

    #[test]
    fn the_first_of_two_loops_is_taken_whatever_its_fraction_and_count() {
        let bytes = with_chunk(
            wav(1_000),
            b"smpl",
            &smpl(&[(1, 100, 500, 0x8000_0000, 3), (0, 10, 20, 0, 0)]),
        );
        let found = read(&bytes, 1_000).expect("the first loop");
        assert_eq!(
            (found.start, found.end, found.alternate),
            (100, 500, true),
            "the second loop, or the fraction, moved the first"
        );
    }

    /// **A chunk list that lies never panics.** Sizes past the end, odd padding, zero-size chunks and
    /// garbage, from a seeded stream so a failure is reproducible.
    #[test]
    fn a_chunk_list_that_lies_never_panics() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        for _ in 0..2_000 {
            let mut bytes = wav(64);
            for _ in 0..(next() % 5) {
                let id: &[u8; 4] = if next() % 2 == 0 { b"smpl" } else { b"junk" };
                let length = (next() % 90) as usize;
                let body: Vec<u8> = (0..length).map(|_| next() as u8).collect();
                bytes = with_chunk(bytes, id, &body);
                if next() % 3 == 0 {
                    let at = bytes.len() - length - 4;
                    let lie = (next() as u32).to_le_bytes();
                    bytes[at..at + 4].copy_from_slice(&lie);
                }
            }
            if next() % 4 == 0 {
                let cut = (next() as usize) % bytes.len();
                bytes.truncate(cut);
            }
            let _ = read(&bytes, 64);
        }
    }

    /// A loop of a harmonic tone at `hz`, `frames` frames long at `rate`.
    fn tone_loop(frames: usize, rate: f32, hz: f32) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|n| {
                let phase = TAU * hz * n as f32 / rate;
                let value = phase.sin() * 0.5 + (2.0 * phase).sin() * 0.2;
                [value, value]
            })
            .collect()
    }

    /// **The loop's own pitch, octave included, and exact to the cent.** One, two, three and four
    /// cycles of middle C at 44.1 kHz land on C4 — where the loop's length alone would say C4, C3,
    /// about F2 and C2 — whether the count is guided by the file's unity note or, with none, by the
    /// detector.
    #[test]
    fn a_loop_holding_several_cycles_lands_on_its_own_note() {
        let rate = 44_100.0;
        for unity in [Some(60.0), None] {
            for (frames, cycles) in [(169usize, 1.0f32), (337, 2.0), (506, 3.0), (675, 4.0)] {
                let hz = rate * cycles / frames as f32;
                let root = loop_pitch(&tone_loop(frames, rate, hz), rate, unity)
                    .root
                    .unwrap_or_else(|| panic!("a {cycles}-cycle loop found no pitch ({unity:?})"));
                let wanted = 69.0 + 12.0 * (hz / 440.0).log2();
                assert!(
                    (root - wanted).abs() < 0.001,
                    "a {cycles}-cycle loop of {frames} frames landed on {root}, not {wanted} \
                     ({unity:?})"
                );
            }
            // Tuned to the cent: 171 frames of one cycle is 25 cents flat of C4.
            let hz = rate / 171.0;
            let root = loop_pitch(&tone_loop(171, rate, hz), rate, unity)
                .root
                .expect("a pitch");
            assert!(
                (root - 59.748).abs() < 0.005,
                "171 frames landed on {root} ({unity:?})"
            );
        }
    }

    /// **The unity note decides the count where the loop's cycles differ.** Two cycles of middle C,
    /// the second quieter: the loop really does repeat only every two cycles, which is what a detector
    /// listening to it repeated can lock onto, an octave low. Counted against the file's unity note,
    /// it lands on C4.
    #[test]
    fn the_unity_note_places_a_loop_whose_cycles_differ() {
        let rate = 44_100.0;
        let loop_frames: Vec<[f32; 2]> = tone_loop(338, rate, rate * 2.0 / 338.0)
            .into_iter()
            .enumerate()
            .map(|(n, [value, _])| {
                let scaled = if n < 169 { value } else { value * 0.6 };
                [scaled, scaled]
            })
            .collect();
        let found = loop_pitch(&loop_frames, rate, Some(60.0));
        let root = found.root.expect("two cycles of C4 are a pitch");
        assert!(
            (root - 59.955).abs() < 0.01,
            "two cycles counted against C4 landed on {root}"
        );
        assert!(
            !found.heard,
            "the detector was asked when the file named its note"
        );
    }

    /// **Nothing else sets a pitch**: a loop slower than twenty times a second, a loop of noise with
    /// no unity note, and a loop that does not hold a whole number of its unity note's cycles.
    #[test]
    fn a_loop_that_is_not_one_note_has_no_pitch() {
        let rate = 44_100.0;
        // 2 400 frames repeats at 18 Hz, below the owner's floor, even though it is a clean tone.
        for unity in [Some(60.0), None] {
            let slow = loop_pitch(&tone_loop(2_400, rate, rate * 8.0 / 2_400.0), rate, unity);
            assert!(slow.repeat_hz < LOOP_PITCH_FLOOR_HZ);
            assert_eq!(
                slow.root, None,
                "a phrase-length loop set a pitch ({unity:?})"
            );
        }

        let mut state = 0x2545_f491_4f6c_dd1du64;
        let noise: Vec<[f32; 2]> = (0..1_500)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let value = (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
                [value, value]
            })
            .collect();
        assert_eq!(
            loop_pitch(&noise, rate, None).root,
            None,
            "noise set a pitch"
        );

        // 816 frames is 4.84 cycles of middle C, the shape of the owner's `069 Strat Atk`.
        let broken = loop_pitch(&tone_loop(816, rate, 261.63), rate, Some(60.0));
        assert_eq!(
            broken.root, None,
            "a loop of 4.84 cycles of its unity note set a pitch: {broken:?}"
        );
    }
}
