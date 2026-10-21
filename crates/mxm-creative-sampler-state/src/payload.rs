//! The embedded audio: a state's `samples` field, checked, decoded and made.
//!
//! **One schema-1 payload, both layers as canonical stereo i16 in base64.** Paths never enter it,
//! and a file's chunks never enter it. The plugin publishes what is read here to its audio thread
//! (`plugins/mxm-creative-sampler/src/asset.rs`, which keeps the publication and its barrier); a
//! state reads and writes it as it is.
//!
//! **Every buffer a payload, a preset or an imported file sizes is reserved fallibly** (audit D14;
//! mxm-kit's `docs/code-review-notes.md` §1): the frame cap bounds a layer, and a cap does not make
//! that allocation succeed. A refusal changes nothing.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mxm_creative_sampler_dsp::{Sample, SampleError};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The most frames a layer holds: two minutes at 48 kHz. **Provisional.**
pub const MAX_FRAMES_PER_LAYER: usize = 5_760_000;

/// What a refused reservation reports. The previous sound is kept, which every caller guarantees.
pub const NOT_ENOUGH_MEMORY: &str =
    "not enough memory for the embedded audio; the previous sound was kept";

/// Reserves exactly `additional` more elements, or refuses.
///
/// **Every buffer sized by a payload, a preset or an imported file is reserved here** (audit D14;
/// mxm-kit's `docs/code-review-notes.md` §1). The frame cap bounds a layer at 43.95 MiB of frames,
/// and a cap does not make that allocation succeed: an infallible one that fails aborts the host.
/// `what` names the buffer for the refusal tests.
pub(crate) fn reserve<T>(
    buffer: &mut Vec<T>,
    additional: usize,
    what: &'static str,
) -> Result<(), String> {
    if refused_for_test(what) {
        return Err(NOT_ENOUGH_MEMORY.to_owned());
    }
    buffer
        .try_reserve_exact(additional)
        .map_err(|_| NOT_ENOUGH_MEMORY.to_owned())
}

/// [`reserve`], for text.
pub(crate) fn reserve_text(
    buffer: &mut String,
    additional: usize,
    what: &'static str,
) -> Result<(), String> {
    if refused_for_test(what) {
        return Err(NOT_ENOUGH_MEMORY.to_owned());
    }
    buffer
        .try_reserve_exact(additional)
        .map_err(|_| NOT_ENOUGH_MEMORY.to_owned())
}

/// Whether a test has asked for this reservation to be refused. Always `false` without the
/// `refusal` feature.
#[cfg(any(test, feature = "refusal"))]
fn refused_for_test(what: &'static str) -> bool {
    refusal::refuses(what)
}

#[cfg(not(any(test, feature = "refusal")))]
fn refused_for_test(_: &'static str) -> bool {
    false
}

/// Refusals injected at the reservation seam, per thread so parallel tests cannot see each other's.
#[cfg(any(test, feature = "refusal"))]
pub mod refusal {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static REFUSE_AT: Cell<Option<usize>> = const { Cell::new(None) };
        static SEEN: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        static REFUSED: Cell<bool> = const { Cell::new(false) };
    }

    /// Starts recording reservations, refusing the `at`-th (from zero) if there is one.
    pub fn arm(at: Option<usize>) {
        ARMED.set(true);
        REFUSE_AT.set(at);
        REFUSED.set(false);
        SEEN.with_borrow_mut(Vec::clear);
    }

    /// Stops refusing; returns every reservation asked for since [`arm`] and whether one was refused.
    pub fn disarm() -> (Vec<&'static str>, bool) {
        ARMED.set(false);
        REFUSE_AT.set(None);
        (SEEN.with_borrow_mut(std::mem::take), REFUSED.replace(false))
    }

    pub(super) fn refuses(what: &'static str) -> bool {
        if !ARMED.get() {
            return false;
        }
        let index = SEEN.with_borrow_mut(|seen| {
            seen.push(what);
            seen.len() - 1
        });
        let refused = REFUSE_AT.get() == Some(index);
        if refused {
            REFUSED.set(true);
        }
        refused
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncodedSample {
    pub name: String,
    pub sample_rate: u32,
    pub frames: u32,
    /// Canonical little-endian interleaved stereo i16, base64 encoded.
    pub pcm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetPayload {
    pub schema: u32,
    pub layers: [Option<EncodedSample>; 2],
}

impl Default for AssetPayload {
    fn default() -> Self {
        Self {
            schema: 1,
            layers: [None, None],
        }
    }
}

/// An [`AssetPayload`] read in place, its text borrowed from the JSON it was read out of.
///
/// **Reading a payload does not copy its audio** (audit D14). Checking a restored state parsed its
/// base64 into owned strings, and a preset cloned its whole JSON value first, before a byte of either
/// was checked. Borrowed, a check copies nothing, and preparation copies only what it keeps, through
/// [`reserve_text`]. A string with an escape in it cannot be borrowed and is copied by the parser;
/// canonical base64 has none.
#[derive(Deserialize)]
pub struct PayloadView<'a> {
    schema: u32,
    #[serde(borrow)]
    layers: [Option<LayerView<'a>>; 2],
}

#[derive(Deserialize)]
struct LayerView<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    sample_rate: u32,
    frames: u32,
    #[serde(borrow)]
    pcm: Cow<'a, str>,
}

impl<'a> PayloadView<'a> {
    /// A payload held in a preset's JSON value.
    pub fn from_value(state: &'a serde_json::Value) -> Result<Self, String> {
        Self::deserialize(state).map_err(|error| format!("invalid embedded sample state: {error}"))
    }

    /// A payload as a state's `samples` field writes it.
    pub fn from_json(serialized: &'a str) -> Result<Self, String> {
        serde_json::from_str(serialized)
            .map_err(|error| format!("invalid embedded sample state: {error}"))
    }

    fn layers(&self) -> [Option<Layer<'_>>; 2] {
        self.layers.each_ref().map(|layer| {
            layer.as_ref().map(|layer| Layer {
                name: &layer.name,
                sample_rate: layer.sample_rate,
                frames: layer.frames,
                pcm: &layer.pcm,
            })
        })
    }

    /// Checks it completely — schema, counts, rate and every byte of its audio — **without a
    /// buffer**.
    pub fn validate(&self) -> Result<(), String> {
        validate_layers(self.schema, self.layers())
    }

    /// The audio it describes, each layer decoded once. See [`prepare`].
    pub fn prepare(&self) -> Result<Prepared, String> {
        prepare_layers(self.schema, self.layers())
    }

    /// The payload to keep, every string of it reserved fallibly.
    pub fn to_payload(&self) -> Result<AssetPayload, String> {
        let [a, b] = self.layers();
        Ok(AssetPayload {
            schema: self.schema,
            layers: [
                a.map(Layer::to_encoded).transpose()?,
                b.map(Layer::to_encoded).transpose()?,
            ],
        })
    }
}

/// One layer's fields, whether an [`EncodedSample`] or a [`PayloadView`] holds them.
#[derive(Clone, Copy)]
struct Layer<'a> {
    name: &'a str,
    sample_rate: u32,
    frames: u32,
    pcm: &'a str,
}

impl Layer<'_> {
    /// An owned copy, every buffer reserved fallibly.
    fn to_encoded(self) -> Result<EncodedSample, String> {
        let mut name = String::new();
        reserve_text(&mut name, self.name.len(), "encoded name")?;
        name.push_str(self.name);
        let mut pcm = String::new();
        reserve_text(&mut pcm, self.pcm.len(), "encoded audio")?;
        pcm.push_str(self.pcm);
        Ok(EncodedSample {
            name,
            sample_rate: self.sample_rate,
            frames: self.frames,
            pcm,
        })
    }
}

impl EncodedSample {
    fn layer(&self) -> Layer<'_> {
        Layer {
            name: &self.name,
            sample_rate: self.sample_rate,
            frames: self.frames,
            pcm: &self.pcm,
        }
    }
}

impl AssetPayload {
    fn layer_refs(&self) -> [Option<Layer<'_>>; 2] {
        self.layers
            .each_ref()
            .map(|layer| layer.as_ref().map(EncodedSample::layer))
    }

    /// A copy without layer `left_out`, every buffer reserved fallibly: the part of the payload a
    /// one-layer change carries across.
    pub fn copy_without(&self, left_out: usize) -> Result<Self, String> {
        let [a, b] = self.layer_refs();
        Ok(Self {
            schema: self.schema,
            layers: [
                a.filter(|_| left_out != 0)
                    .map(Layer::to_encoded)
                    .transpose()?,
                b.filter(|_| left_out != 1)
                    .map(Layer::to_encoded)
                    .transpose()?,
            ],
        })
    }

    /// A cheap identity for the payload, which a loaded preset compares to tell whether its audio
    /// changed: FNV-1a over every field.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut add = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        add(&self.schema.to_le_bytes());
        for layer in &self.layers {
            match layer {
                None => add(&[0]),
                Some(sample) => {
                    add(&[1]);
                    add(&sample.sample_rate.to_le_bytes());
                    add(&sample.frames.to_le_bytes());
                    add(sample.name.as_bytes());
                    add(&[0xff]);
                    add(sample.pcm.as_bytes());
                }
            }
        }
        hash
    }
}

/// Checks a payload completely — schema, counts, rate and every byte of its audio — **without a
/// buffer**. See [`decode_frames`].
pub fn validate_payload(payload: &AssetPayload) -> Result<(), String> {
    validate_layers(payload.schema, payload.layer_refs())
}

/// [`validate_payload`] on a serialized `samples` field, read in place, for `Plugin::filter_state`:
/// checking a restored state copies none of its audio.
pub fn validate_serialized_payload(serialized: &str) -> Result<(), String> {
    PayloadView::from_json(serialized)?.validate()
}

fn validate_layers(schema: u32, layers: [Option<Layer<'_>>; 2]) -> Result<(), String> {
    check_schema(schema)?;
    for layer in layers.into_iter().flatten() {
        let frames = check_layer(layer)?;
        decode_frames(layer, frames, |_| {})?;
    }
    Ok(())
}

fn check_schema(schema: u32) -> Result<(), String> {
    if schema == 1 {
        Ok(())
    } else {
        Err(format!("unsupported sample-state schema {schema}"))
    }
}

/// The most of a name an error message quotes. A name is external text of any length, and a message
/// that copied all of it would be one more buffer the payload sized.
const QUOTED_NAME_BYTES: usize = 120;

fn quoted(name: &str) -> &str {
    let mut end = name.len().min(QUOTED_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// A layer's frame count, once its count, rate and encoded length agree, before a byte is decoded.
fn check_layer(layer: Layer<'_>) -> Result<usize, String> {
    let frames = layer.frames as usize;
    if frames == 0 || frames > MAX_FRAMES_PER_LAYER {
        return Err(format!(
            "{} has an invalid or oversized frame count",
            quoted(layer.name)
        ));
    }
    if !(1_000..=768_000).contains(&layer.sample_rate) {
        return Err(format!("{} has an invalid sample rate", quoted(layer.name)));
    }
    // Canonical padded base64 of `4 × frames` bytes has exactly this length; the engine refuses a
    // string that omits its padding, and any other length decodes to a different byte count.
    if layer.pcm.len() != (frames * 4).div_ceil(3) * 4 {
        return Err(format!(
            "{} has inconsistent embedded audio",
            quoted(layer.name)
        ));
    }
    Ok(frames)
}

/// Base64 characters decoded at a time: whole quads, which decode to whole frames.
const DECODE_CHARS: usize = 4_096;
const DECODE_BYTES: usize = DECODE_CHARS / 4 * 3;

/// Decodes a checked layer's audio a chunk at a time and hands `each` every canonical frame.
///
/// **One decode, into no buffer of its own** (audit D14). Validation decoded the whole string into a
/// vector it then threw away — 21.97 MiB at the frame cap, reserved infallibly — and preparation
/// decoded it again before copying it into frames. A fixed stack chunk serves both: validation
/// discards the frames, preparation pushes them into a vector [`reserve`]d once.
///
/// It accepts exactly what one whole-string decode of the right length does. A chunk decodes whole
/// quads, so padding can only shorten its last quad, by one or two bytes: a chunk before the last
/// that held padding — which the whole decode refuses — decodes off the four-byte frame grid and is
/// refused here too. The running total must stay within the layer's bytes, checked before a chunk's
/// frames are handed over, so a caller's reservation is never exceeded.
fn decode_frames(
    layer: Layer<'_>,
    frames: usize,
    mut each: impl FnMut(&[u8; 4]),
) -> Result<(), String> {
    let expected = frames * 4;
    let invalid = || format!("{} contains invalid embedded audio", quoted(layer.name));
    let truncated = || format!("{} contains truncated embedded audio", quoted(layer.name));
    let mut bytes = [0u8; DECODE_BYTES];
    let mut decoded = 0;
    for chunk in layer.pcm.as_bytes().chunks(DECODE_CHARS) {
        let written = BASE64
            .decode_slice(chunk, &mut bytes)
            .map_err(|_| invalid())?;
        decoded += written;
        let (whole, partial) = bytes[..written].as_chunks::<4>();
        if decoded > expected || !partial.is_empty() {
            return Err(truncated());
        }
        whole.iter().for_each(&mut each);
    }
    if decoded == expected {
        Ok(())
    } else {
        Err(truncated())
    }
}

/// One canonical stereo i16 frame, as a layer plays it.
///
/// **The one place the reconstruction lives**, so what an import measures and what a layer plays
/// cannot drift apart: a loop's pitch was once heard in the decoded floats, and quantisation can move
/// a quiet loop's peak under the detector's level floor (code review round 2).
#[must_use]
pub fn canonical_frame(bytes: &[u8; 4]) -> [f32; 2] {
    [
        i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0,
        i16::from_le_bytes([bytes[2], bytes[3]]) as f32 / 32_768.0,
    ]
}

/// The audio a payload describes: each layer as the DSP plays it, and its name.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub samples: [Option<Sample>; 2],
    pub names: [Option<String>; 2],
}

/// The audio a payload describes, **each layer decoded once, into frames reserved fallibly**
/// (audit D14).
///
/// This checks everything [`validate_payload`] checks, as it goes, so it does not call it: that
/// decode was a second pass over the same audio. Any refusal — malformed audio, or memory the
/// frames, the name or the sample's own analysis cannot have — returns before anything is staged
/// or published, so the caller keeps the sound it had.
pub fn prepare(payload: &AssetPayload) -> Result<Prepared, String> {
    prepare_layers(payload.schema, payload.layer_refs())
}

/// One layer as the DSP plays it: an arriving source, whose name's octave is heard in it.
pub fn prepare_sample(sample: &EncodedSample) -> Result<Sample, String> {
    let [prepared, _] = prepare_layers(1, [Some(sample.layer()), None])?.samples;
    Ok(prepared.expect("the one layer given is prepared"))
}

fn prepare_layers(schema: u32, layers: [Option<Layer<'_>>; 2]) -> Result<Prepared, String> {
    check_schema(schema)?;
    let mut prepared = Prepared::default();
    for (index, layer) in layers.into_iter().enumerate() {
        let Some(layer) = layer else {
            continue;
        };
        let frames = check_layer(layer)?;
        let mut decoded: Vec<[f32; 2]> = Vec::new();
        reserve(&mut decoded, frames, "frames")?;
        decode_frames(layer, frames, |bytes| decoded.push(canonical_frame(bytes)))?;
        prepared.samples[index] = Some(Sample::new(decoded, layer.sample_rate as f32).map_err(
            |error| match error {
                SampleError::Allocation => NOT_ENOUGH_MEMORY.to_owned(),
                other => format!("{} could not be prepared: {other:?}", quoted(layer.name)),
            },
        )?);
        let mut name = String::new();
        reserve_text(&mut name, layer.name.len(), "name")?;
        name.push_str(layer.name);
        prepared.names[index] = Some(name);
    }
    Ok(prepared)
}

/// The name the generated starting source carries, in the browser and in saved state.
///
/// **It ends in `C3`**, which is not decoration: the arrival reads a trailing key off a loaded
/// source's name and tunes Root from it, so every generated source arrives in tune through the path
/// a named file already uses. See [`crate::generate::Recipe::name`].
#[must_use]
pub fn default_source_name() -> String {
    default_source_recipe().name()
}

/// The MIDI note that plays any generated source at its recorded rate. C3.
pub const DEFAULT_SOURCE_ROOT: f32 = crate::generate::ROOT;

/// The source the instrument starts on: a band-limited saw, generated rather than shipped.
///
/// **An instrument with nothing loaded cannot be played, and four of the plugin's factory recipes
/// carry no audio of their own** — they are `state = null` by design, so they treat whatever is
/// loaded. With an empty bank they treated nothing. This is the sound they treat out of the box.
///
/// It is now one of seventeen ([`crate::generate::Recipe`]) rather than the only one, and this is the
/// entry that does not move — the owner-selected init patch is tuned against it.
#[must_use]
pub fn default_source() -> EncodedSample {
    encode_generated(default_source_recipe())
}

fn default_source_recipe() -> crate::generate::Recipe {
    crate::init::PATCH.sources[0].expect("Init must provide the one audible source on layer A")
}

/// Render a recipe and wrap it in the payload's canonical encoding.
///
/// Project-owned by construction: additive synthesis from a named technique, so there is no
/// third-party audio in the repository and no binary in the tree.
#[must_use]
pub fn encode_generated(recipe: crate::generate::Recipe) -> EncodedSample {
    let frames = crate::generate::render(recipe);
    let mut pcm = Vec::with_capacity(frames.len() * 4);
    for value in &frames {
        let scaled = (value * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16;
        // Mono content in the canonical stereo payload: the width is the instrument's to make,
        // not the source's to bake in.
        pcm.extend_from_slice(&scaled.to_le_bytes());
        pcm.extend_from_slice(&scaled.to_le_bytes());
    }
    EncodedSample {
        name: recipe.name(),
        sample_rate: crate::generate::RATE,
        frames: frames.len() as u32,
        pcm: BASE64.encode(pcm),
    }
}

/// The payload a fresh instance starts with: the generated saw on A, nothing on B.
#[must_use]
pub fn initial_payload() -> AssetPayload {
    AssetPayload {
        schema: 1,
        layers: crate::init::PATCH
            .sources
            .map(|recipe| recipe.map(encode_generated)),
    }
}

/// Canonical stereo i16 from decoded floats, as an import quantises once: each sample clamped to
/// `-1..=1`, scaled by 32 767 and rounded; the first two channels kept and mono duplicated.
///
/// `interleaved` is clamped where it lies. Every buffer sized here is reserved fallibly.
pub(crate) fn quantise(
    interleaved: &mut [f32],
    channels: usize,
    frames: usize,
) -> Result<Vec<u8>, String> {
    for value in interleaved.iter_mut() {
        *value = value.clamp(-1.0, 1.0);
    }
    let mut pcm = Vec::new();
    reserve(&mut pcm, frames * 4, "import audio")?;
    for frame in interleaved.chunks_exact(channels) {
        let left = frame[0];
        let right = if channels == 1 { left } else { frame[1] };
        let left = (left * 32_767.0).round() as i16;
        let right = (right * 32_767.0).round() as i16;
        pcm.extend_from_slice(&left.to_le_bytes());
        pcm.extend_from_slice(&right.to_le_bytes());
    }
    Ok(pcm)
}

/// `pcm` in the payload's base64, reserved fallibly.
pub(crate) fn encode_pcm(pcm: &[u8]) -> Result<String, String> {
    let mut encoded = String::new();
    let encoded_len = base64::encoded_len(pcm.len(), true).ok_or(NOT_ENOUGH_MEMORY)?;
    reserve_text(&mut encoded, encoded_len, "import encoding")?;
    BASE64.encode_string(pcm, &mut encoded);
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(value: i16) -> AssetPayload {
        let mut bytes = Vec::new();
        for _ in 0..16 {
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes.extend_from_slice(&(-value).to_le_bytes());
        }
        AssetPayload {
            schema: 1,
            layers: [
                Some(EncodedSample {
                    name: "portable.wav".to_owned(),
                    sample_rate: 48_000,
                    frames: 16,
                    pcm: BASE64.encode(bytes),
                }),
                None,
            ],
        }
    }

    /// Two layers of `frames` frames each, every frame distinct from its neighbours and from
    /// another seed's. 2 000 frames spans three of the decoder's chunks.
    fn layered(seed: i16, frames: usize) -> AssetPayload {
        let layer = |offset: i16, name: &str| {
            let mut bytes = Vec::with_capacity(frames * 4);
            for frame in 0..frames {
                let value = seed.wrapping_add(offset).wrapping_add(frame as i16);
                bytes.extend_from_slice(&value.to_le_bytes());
                bytes.extend_from_slice(&value.wrapping_neg().to_le_bytes());
            }
            EncodedSample {
                name: name.to_owned(),
                sample_rate: 48_000,
                frames: frames as u32,
                pcm: BASE64.encode(bytes),
            }
        };
        AssetPayload {
            schema: 1,
            layers: [Some(layer(0, "left.wav")), Some(layer(7, "right.wav"))],
        }
    }

    #[test]
    fn payload_round_trip_builds_the_same_canonical_samples() {
        let payload = payload(12_345);
        let prepared = prepare(&payload).unwrap();
        let sample = prepared.samples[0].as_ref().unwrap();
        assert_eq!(
            sample.frames()[0],
            [12_345.0 / 32_768.0, -12_345.0 / 32_768.0]
        );
    }

    /// The generated saw loops without a click and sits exactly on the note Root is set to.
    ///
    /// Both properties come from the same choice of frame and cycle counts, and neither survives
    /// rounding them to something tidier — which is why they are asserted rather than trusted.
    #[test]
    fn the_generated_source_loops_seamlessly_and_is_in_tune() {
        let encoded = default_source();
        let payload = AssetPayload {
            schema: 1,
            layers: [Some(encoded), None],
        };
        let prepared = prepare(&payload).expect("the generated source decodes");
        let sample = prepared.samples[0].as_ref().expect("a sample");
        let frames = sample.frames();

        // Seamless: the wrap from the last frame to the first is no larger a step than the
        // waveform takes anywhere else in its cycle.
        let cycle = crate::generate::FRAMES / crate::generate::CYCLES;
        let largest_internal = frames
            .windows(2)
            .map(|pair| (pair[1][0] - pair[0][0]).abs())
            .fold(0.0f32, f32::max);
        let wrap = (frames[0][0] - frames[frames.len() - 1][0]).abs();
        assert!(
            wrap <= largest_internal,
            "the loop point steps further than the waveform does: wrap {wrap} \
             against {largest_internal} over a {cycle}-frame cycle"
        );

        // In tune: the fundamental implied by the frame and cycle counts is the root, to a cent.
        let fundamental = crate::generate::RATE as f32 * crate::generate::CYCLES as f32
            / crate::generate::FRAMES as f32;
        let midi = 69.0 + 12.0 * (fundamental / 440.0).log2();
        assert!(
            (midi - DEFAULT_SOURCE_ROOT).abs() < 0.01,
            "the generated source is not on its declared root: {midi} against {DEFAULT_SOURCE_ROOT}"
        );

        // Band-limited, measured rather than inferred from the slope. The buffer holds a whole
        // number of cycles, so harmonic `n` lands exactly on DFT bin `n * cycles` and the
        // magnitude there is the partial's own amplitude with no leakage to argue about.
        let harmonic = |n: usize| {
            let mut real = 0.0f64;
            let mut imaginary = 0.0f64;
            for (index, frame) in frames.iter().enumerate() {
                let phase = -std::f64::consts::TAU
                    * n as f64
                    * crate::generate::CYCLES as f64
                    * index as f64
                    / crate::generate::FRAMES as f64;
                real += f64::from(frame[0]) * phase.cos();
                imaginary += f64::from(frame[0]) * phase.sin();
            }
            (real.hypot(imaginary) / crate::generate::FRAMES as f64) as f32
        };
        let fundamental_level = harmonic(1);
        let last_kept = (crate::generate::BANDLIMIT_HZ / fundamental).floor() as usize;
        assert!(
            fundamental_level > 0.1,
            "the fundamental is missing: {fundamental_level}"
        );
        // A saw falls as 1/n, so the last kept partial is small but must be there.
        assert!(
            harmonic(last_kept) > fundamental_level / (last_kept as f32 * 4.0),
            "the band limit cuts in early: partial {last_kept} is {}",
            harmonic(last_kept)
        );
        // And the first one past it must be gone outright, or an octave up folds.
        assert!(
            harmonic(last_kept + 1) < fundamental_level * 1.0e-4,
            "content past the band limit: partial {} is {}",
            last_kept + 1,
            harmonic(last_kept + 1)
        );
    }

    /// **The detector and the generated source agree about what note it is**, independently.
    ///
    /// `DEFAULT_SOURCE_ROOT` is asserted elsewhere from the frame and cycle counts — arithmetic on
    /// how the saw was built. This asks the question the other way round, from the audio, with the
    /// same code the Auto button runs. Two derivations meeting is worth more than either alone: if
    /// the generator is ever retuned and the constant is not, or the detector drifts, this fails.
    #[test]
    fn the_generated_source_is_heard_as_the_root_it_declares() {
        let payload = AssetPayload {
            schema: 1,
            layers: [Some(default_source()), None],
        };
        let prepared = prepare(&payload).expect("the generated source decodes");
        let sample = prepared.samples[0].as_ref().expect("a sample");
        let heard = mxm_creative_sampler_dsp::detect_root(sample, 0.0)
            .expect("the generated saw has a pitch");
        assert!(
            (heard - DEFAULT_SOURCE_ROOT).abs() < 0.2,
            "the saw declares MIDI {DEFAULT_SOURCE_ROOT} and sounds like {heard}"
        );
    }

    /// **Every one of the seventeen survives the round trip through its payload without a seam.**
    ///
    /// The analytic test in `generate` evaluates an idealised `f64` expression, and the spectral
    /// one there measures `render`'s `f32` output — but neither sees what encoding to i16 and
    /// decoding back does, which is the form a player is handed. This runs the same seam
    /// measurement on the decoded frames, for all of them rather than only the init saw.
    #[test]
    fn every_generated_source_survives_its_payload_without_a_seam() {
        for recipe in crate::generate::Recipe::ALL {
            let payload = AssetPayload {
                schema: 1,
                layers: [Some(encode_generated(recipe)), None],
            };
            let prepared = prepare(&payload).expect("the generated source decodes");
            let sample = prepared.samples[0].as_ref().expect("a sample");
            let mono: Vec<f32> = sample.frames().iter().map(|frame| frame[0]).collect();
            let seam = crate::generate::seam_out_of_band_db(&mono);
            assert!(
                seam < -55.0,
                "{} has a {seam:.1} dB seam once it has been through its payload",
                recipe.label()
            );
        }
    }

    #[test]
    fn malformed_payloads_are_rejected_before_publication() {
        let mut bad = payload(1);
        bad.layers[0].as_mut().unwrap().pcm.truncate(4);
        assert!(validate_payload(&bad).is_err());
        assert!(prepare(&bad).is_err());

        let mut same_length_garbage = payload(1);
        let length = same_length_garbage.layers[0].as_ref().unwrap().pcm.len();
        same_length_garbage.layers[0].as_mut().unwrap().pcm = "!".repeat(length);
        assert!(validate_payload(&same_length_garbage).is_err());
    }

    /// **The chunked decoder accepts exactly what one whole-string decode accepts, and reads the
    /// same frames.** It decodes a layer a few thousand characters at a time so validation needs no
    /// buffer and preparation needs only the frames; a chunk that decoded short in the middle of the
    /// stream (padding where none may be) would otherwise shift every frame after it.
    #[test]
    fn the_chunked_decode_accepts_exactly_what_a_whole_decode_accepts() {
        for frames in [1usize, 767, 768, 769, 1_536, 2_000] {
            let good = layered(3, frames).layers[0].clone().unwrap();
            let length = good.pcm.len();
            let mut cases = vec![good.pcm.clone()];
            let mut replaced = |at: usize, with: &str| {
                if at + with.len() <= length {
                    let mut pcm = good.pcm.clone();
                    pcm.replace_range(at..at + with.len(), with);
                    cases.push(pcm);
                }
            };
            // Padding closing the first chunk, a bad character in each place, and the last symbol
            // carrying bits the padding says are not there.
            replaced(4_096 - 1, "=");
            replaced(4_096 - 2, "==");
            replaced(0, "!");
            replaced(5_000, "*");
            replaced(length - 1, "A");
            replaced(length - 2, "AA");
            if good.pcm.ends_with("==") {
                replaced(length - 3, "B");
            } else if good.pcm.ends_with('=') {
                replaced(length - 2, "B");
            }
            // Padding closing the first chunk, paid for by filling in the padding at the end, so the
            // byte total still matches and only the frame grid can tell. The symbol before the
            // padding carries no stray bits, or the decode would refuse it for that instead.
            let pad = length - good.pcm.trim_end_matches('=').len();
            if length > 4_096 && pad > 0 {
                let mut pcm = good.pcm.clone();
                pcm.replace_range(4_096 - pad - 1..4_096, &format!("A{}", "=".repeat(pad)));
                pcm.replace_range(length - pad..length, &"A".repeat(pad));
                cases.push(pcm);
            }
            let mut short = good.pcm.clone();
            short.truncate(length - 4);
            cases.push(short);
            cases.push(format!("{}AAAA", good.pcm));

            for pcm in cases {
                let whole = BASE64
                    .decode(&pcm)
                    .ok()
                    .filter(|bytes| bytes.len() == frames * 4);
                let payload = AssetPayload {
                    schema: 1,
                    layers: [
                        Some(EncodedSample {
                            pcm: pcm.clone(),
                            ..good.clone()
                        }),
                        None,
                    ],
                };
                let tail = &pcm[pcm.len().saturating_sub(8)..];
                assert_eq!(
                    validate_payload(&payload).is_ok(),
                    whole.is_some(),
                    "{frames} frames ending {tail}: validation disagrees with a whole decode"
                );
                match (prepare(&payload), whole) {
                    (Ok(prepared), Some(bytes)) => {
                        let expected: Vec<[f32; 2]> = bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(canonical_frame)
                            .collect();
                        assert_eq!(
                            prepared.samples[0].as_ref().unwrap().frames(),
                            &expected[..],
                            "{frames} frames: the chunked decode read different frames"
                        );
                    }
                    (Err(_), None) => {}
                    (prepared, whole) => panic!(
                        "{frames} frames ending {tail}: preparation {} where a whole decode {}",
                        if prepared.is_ok() {
                            "accepted"
                        } else {
                            "refused"
                        },
                        if whole.is_some() {
                            "accepted"
                        } else {
                            "refused"
                        }
                    ),
                }
            }
        }
    }
}
