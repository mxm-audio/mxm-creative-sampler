//! The plugin's state: every parameter's value, the embedded audio and the loaded preset's identity,
//! read from and written to the JSON and the CLAP stream the plugin itself reads and writes.
//!
//! **What the plugin writes reads here, and writes back equal**, byte for byte: nice-plug's
//! `PluginState` as `serde_json` writes it — `version`, then `params` by id, sorted, each
//! `{"f32": …}`, `{"bool": …}` or a choice's `{"string": <id>}`, then `fields`, the persisted
//! `preset` and `samples`, each itself a JSON string. A CLAP state stream (what newDAWn stores in a
//! tutti project's `plugins/states/<sha-256>.clap`) is that JSON after its length, eight bytes,
//! little-endian.
//!
//! **A state reads as the plugin loads one into a fresh instance** (nice-plug's
//! `deserialize_object`, after the plugin's `filter_state`):
//!
//! - a parameter the state leaves out keeps its default, and a tempo sync left out is off;
//! - a parameter this plugin does not have, or a value of the wrong kind, is ignored;
//! - a choice is read by its id, or by its index;
//! - `samples` left out keeps the generated Init saw, and `preset` left out is no preset;
//! - **what the plugin refuses whole is an error here**: malformed embedded audio, a `preset` that is
//!   no preset identity, a field the plugin does not keep, and a field that would not write back as
//!   it was read. The plugin keeps the sound it had for each; a program reading a state says why.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use mxm_creative_sampler_dsp::Params;
use mxm_creative_sampler_dsp::routing::Routing;
use serde::{Deserialize, Serialize};

use crate::PLUGIN_VERSION;
use crate::generate::Recipe;
use crate::import::{self, Arrival, Imported};
use crate::params::{self, Kind, LAYERS, TEMPO_SYNC_IDS, TEMPO_SYNCS, id};
use crate::payload::{self, AssetPayload, EncodedSample, PayloadView, Prepared};
use crate::routes::ROUTE_IDS;
use crate::sound::{
    self, CONVERTER_TYPES, FILTER_MODES, LFO_SHAPES, LOOP_MODES, READERS, Sound, VOICE_MODES,
};
use crate::text::DEFAULT_SOURCE_RATE;

/// The persisted field holding the embedded audio ([`AssetPayload`]).
pub const SAMPLES_FIELD: &str = "samples";

/// The persisted field holding the loaded preset's identity ([`PresetIdentity`]).
pub const PRESET_FIELD: &str = "preset";

/// The largest state a CLAP stream may declare: nice-plug's own bound, 256 MiB, which its
/// `ext_state_load` checks before it reads.
pub const MAX_STATE_BYTES: u64 = 268_435_456;

/// Why a state was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// Not a CLAP state stream: shorter than the length it declares, or longer than the plugin
    /// reads.
    Stream(String),
    /// Not the plugin's JSON.
    Json(String),
    /// The embedded audio is malformed or cannot be held.
    Samples(String),
    /// The `preset` field is not a preset identity.
    Preset(String),
    /// A persisted field this plugin does not keep, which makes the plugin refuse the state.
    UnknownField(String),
    /// A persisted field that would not write back as it was read, which makes the plugin refuse
    /// the state.
    NotKept(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Stream(why) => write!(f, "not a CLAP state stream: {why}"),
            StateError::Json(why) => write!(f, "not the sampler's state: {why}"),
            StateError::Samples(why) => write!(f, "the embedded audio was refused: {why}"),
            StateError::Preset(why) => write!(f, "the preset identity was refused: {why}"),
            StateError::UnknownField(key) => write!(f, "the state holds an unknown field, {key}"),
            StateError::NotKept(key) => {
                write!(
                    f,
                    "the state's field {key} would not be kept as it was written"
                )
            }
        }
    }
}

impl std::error::Error for StateError {}

/// A plain, unnormalised value as nice-plug stores one. Only a choice is stored as a `string`, its
/// variant's id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ParamValue {
    F32(f32),
    I32(i32),
    Bool(bool),
    String(String),
}

/// nice-plug's `PluginState`, as read. Its `version` is dropped, as the plugin drops it: a state
/// is written with the version that writes it.
#[derive(Deserialize)]
struct Read {
    params: BTreeMap<String, ParamValue>,
    fields: BTreeMap<String, String>,
}

/// nice-plug's `PluginState`, as written: the same fields in the same order.
#[derive(Serialize)]
struct Written<'a> {
    version: &'a str,
    params: BTreeMap<&'a str, ParamValue>,
    fields: BTreeMap<&'a str, String>,
}

/// The loaded preset's identity: mxm-kit's `mxm_preset::PresetIdentity`, the shape its `preset` field
/// is written in. **Kept so a state writes back whole**; nothing here reads it but to keep it. The
/// plugin's tests hold this shape to the kit's.
///
/// *No preset* when `loaded` is `None`; the preset's name otherwise, marked modified in the plugin
/// where the parameters or the audio differ from `baseline` and `state_fingerprint`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresetIdentity {
    #[serde(default = "identity_version")]
    pub version: u32,
    #[serde(default)]
    pub loaded: Option<LoadedPreset>,
    /// Every parameter as the preset left it, normalised, by id.
    #[serde(default)]
    pub baseline: BTreeMap<String, f32>,
    /// The audio as the preset left it: [`AssetPayload::fingerprint`].
    #[serde(default)]
    pub state_fingerprint: Option<u64>,
}

/// Which preset, and from where.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadedPreset {
    pub name: String,
    pub origin: Origin,
}

/// Where a preset came from: compiled in, the person's own, or a bank's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Factory,
    User,
    Bank(String),
}

const fn identity_version() -> u32 {
    1
}

impl PresetIdentity {
    /// No preset: what a fresh instance holds, and what Init returns it to.
    #[must_use]
    pub fn none() -> Self {
        Self {
            version: identity_version(),
            loaded: None,
            baseline: BTreeMap::new(),
            state_fingerprint: None,
        }
    }
}

/// One sampler's state.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Every parameter's plain value, in [`params::all`] order.
    values: Vec<f32>,
    samples: AssetPayload,
    preset: PresetIdentity,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Where each id sits in [`params::all`].
static INDEX: LazyLock<BTreeMap<&'static str, usize>> = LazyLock::new(|| {
    params::all()
        .iter()
        .enumerate()
        .map(|(index, param)| (param.id, index))
        .collect()
});

fn defaults() -> Vec<f32> {
    params::all()
        .iter()
        .map(params::Param::default_plain)
        .collect()
}

impl State {
    /// **A fresh instance**: every parameter at Init, the generated Init saw on layer A, no preset.
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: defaults(),
            samples: payload::initial_payload(),
            preset: PresetIdentity::none(),
        }
    }

    /// A state from the plugin's JSON.
    pub fn from_json(json: &[u8]) -> Result<Self, StateError> {
        let mut read: Read =
            serde_json::from_slice(json).map_err(|error| StateError::Json(error.to_string()))?;

        // `filter_state`: a state written before the tempo syncs restores them off, and a loaded
        // preset's baseline gains them at zero, so an unchanged preset does not reopen modified
        // (`mxm_preset::add_switches_off`).
        for sync in TEMPO_SYNC_IDS {
            read.params
                .entry(sync.to_owned())
                .or_insert(ParamValue::Bool(false));
        }
        if let Some(serialized) = read.fields.get_mut(PRESET_FIELD)
            && let Ok(mut identity) = serde_json::from_str::<PresetIdentity>(serialized)
            && identity.loaded.is_some()
            && !TEMPO_SYNC_IDS
                .iter()
                .all(|sync| identity.baseline.contains_key(*sync))
        {
            for sync in TEMPO_SYNC_IDS {
                identity.baseline.entry(sync.to_owned()).or_insert(0.0);
            }
            if let Ok(canonical) = serde_json::to_string(&identity) {
                *serialized = canonical;
            }
        }
        // The embedded audio is checked in place, before anything is read: a malformed payload
        // makes the plugin's load a complete no-op.
        if let Some(serialized) = read.fields.get(SAMPLES_FIELD) {
            payload::validate_serialized_payload(serialized).map_err(StateError::Samples)?;
        }

        let mut values = defaults();
        for (id, value) in &read.params {
            if let Some(&index) = INDEX.get(id.as_str())
                && let Some(plain) = restored(&params::all()[index].kind, value)
            {
                values[index] = plain;
            }
        }

        let mut samples = None;
        let mut preset = None;
        for (key, requested) in &read.fields {
            let restored = match key.as_str() {
                SAMPLES_FIELD => {
                    let read = PayloadView::from_json(requested)
                        .and_then(|view| view.to_payload())
                        .map_err(StateError::Samples)?;
                    let written = serialize(&read);
                    samples = Some(read);
                    written
                }
                PRESET_FIELD => {
                    let read = serde_json::from_str::<PresetIdentity>(requested)
                        .map_err(|error| StateError::Preset(error.to_string()))?;
                    let written = serialize(&read);
                    preset = Some(read);
                    written
                }
                other => return Err(StateError::UnknownField(other.to_owned())),
            };
            // nice-plug keeps a field only where it writes back what was asked of it.
            if !kept_as_read(requested, &restored) {
                return Err(StateError::NotKept(key.clone()));
            }
        }

        Ok(Self {
            values,
            samples: samples.unwrap_or_else(payload::initial_payload),
            preset: preset.unwrap_or_else(PresetIdentity::none),
        })
    }

    /// The plugin's JSON for this state, **as the plugin writes it**: its version, every
    /// parameter, then the two persisted fields.
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        let params = params::all()
            .iter()
            .zip(&self.values)
            .map(|(param, &plain)| {
                let value = match &param.kind {
                    Kind::Float(_) => ParamValue::F32(plain),
                    Kind::Switch(_) => ParamValue::Bool(plain > 0.5),
                    Kind::Choice(choice) => {
                        let index = (plain.max(0.0) as usize).min(choice.variants.len() - 1);
                        ParamValue::String(choice.variants[index].id.to_owned())
                    }
                };
                (param.id, value)
            })
            .collect();
        let fields = BTreeMap::from([
            (PRESET_FIELD, serialize(&self.preset)),
            (SAMPLES_FIELD, serialize(&self.samples)),
        ]);
        serde_json::to_vec(&Written {
            version: PLUGIN_VERSION,
            params,
            fields,
        })
        .expect("a state always serialises")
    }

    /// A state from a CLAP state stream: its JSON's length as eight little-endian bytes, then the
    /// JSON. Bytes past the declared length are not read, as the plugin reads none.
    pub fn from_clap(stream: &[u8]) -> Result<Self, StateError> {
        let (length, rest) = stream
            .split_first_chunk::<8>()
            .ok_or_else(|| StateError::Stream("no length".to_owned()))?;
        let length = u64::from_le_bytes(*length);
        if length > MAX_STATE_BYTES {
            return Err(StateError::Stream(format!(
                "it declares {length} bytes, past the plugin's {MAX_STATE_BYTES}"
            )));
        }
        let json = rest.get(..length as usize).ok_or_else(|| {
            StateError::Stream(format!(
                "it declares {length} bytes and holds {}",
                rest.len()
            ))
        })?;
        Self::from_json(json)
    }

    /// This state as a CLAP state stream, as the plugin saves one.
    #[must_use]
    pub fn to_clap(&self) -> Vec<u8> {
        let json = self.to_json();
        let mut stream = Vec::with_capacity(8 + json.len());
        stream.extend_from_slice(&(json.len() as u64).to_le_bytes());
        stream.extend_from_slice(&json);
        stream
    }

    /// Every parameter's plain value, in [`params::all`] order.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// `id`'s plain value.
    #[must_use]
    pub fn value(&self, id: &str) -> Option<f32> {
        INDEX.get(id).map(|&index| self.values[index])
    }

    /// Sets `id`'s plain value as given, as a host restoring a plain value does. Returns whether
    /// `id` is a parameter.
    pub fn set_value(&mut self, id: &str, plain: f32) -> bool {
        let Some(&index) = INDEX.get(id) else {
            return false;
        };
        self.values[index] = plain;
        true
    }

    /// `id`'s value as the host's `0..=1`.
    #[must_use]
    pub fn normalized(&self, id: &str) -> Option<f32> {
        let &index = INDEX.get(id)?;
        Some(params::all()[index].normalized(self.values[index]))
    }

    /// Sets `id` from the host's `0..=1`: **what the plugin keeps when a host sets it**, the plain
    /// value the travel gives, snapped where it steps. Returns whether `id` is a parameter.
    pub fn set_normalized(&mut self, id: &str, normalized: f32) -> bool {
        let Some(&index) = INDEX.get(id) else {
            return false;
        };
        self.values[index] = params::all()[index].plain(normalized);
        true
    }

    /// **`id`'s value as a host shows it**, character for character: the text of its normalised
    /// value, against this state's source rate.
    #[must_use]
    pub fn text(&self, id: &str) -> Option<String> {
        let &index = INDEX.get(id)?;
        let param = &params::all()[index];
        Some(param.text(param.normalized(self.values[index]), self.source_rate()))
    }

    /// Sets `id` from typed text, as a host's `text_to_value` and then a gesture do. Returns whether
    /// the text was read.
    pub fn type_text(&mut self, id: &str, text: &str) -> bool {
        let Some(param) = params::get(id) else {
            return false;
        };
        match param.parse(text, self.source_rate()) {
            Some(normalized) => self.set_normalized(id, normalized),
            None => false,
        }
    }

    /// The rate the converter's Sample rate is quoted against: **the first loaded layer's**, and
    /// [`DEFAULT_SOURCE_RATE`] with neither loaded.
    ///
    /// The plugin publishes the first loaded layer's rate from each callback and keeps the last one
    /// it published while both layers are empty; a state has no last one, so it falls back to the
    /// rate a fresh instance starts at.
    #[must_use]
    pub fn source_rate(&self) -> u32 {
        self.samples.layers[0]
            .as_ref()
            .or(self.samples.layers[1].as_ref())
            .map_or(DEFAULT_SOURCE_RATE, |sample| sample.sample_rate)
    }

    /// The embedded audio, as stored.
    #[must_use]
    pub fn samples(&self) -> &AssetPayload {
        &self.samples
    }

    /// The embedded audio as the DSP plays it, each layer decoded and analysed once. Every buffer
    /// is reserved fallibly, so this can refuse for want of memory, as the plugin's restore can.
    pub fn prepare(&self) -> Result<Prepared, String> {
        payload::prepare(&self.samples)
    }

    /// The loaded preset's identity.
    #[must_use]
    pub fn preset(&self) -> &PresetIdentity {
        &self.preset
    }

    /// The values as the DSP reads them.
    fn view(&self) -> Values<'_> {
        Values(&self.values)
    }

    /// Every value the DSP reads but the routes, as this state holds them.
    #[must_use]
    pub fn sound(&self) -> Sound {
        self.view().sound()
    }

    /// **What the DSP plays for this state** at `tempo`, in beats a minute (`None` with no
    /// tempo): the plugin's own mapping, with each tempo sync resolved as the plugin resolves it
    /// once a callback.
    #[must_use]
    pub fn dsp_params(&self, tempo: Option<f64>) -> Params {
        self.view().dsp_params(tempo)
    }

    /// **The routing the DSP plays for this state**: each pair's presence, a present route's
    /// amount as stored, and the topology compacted.
    #[must_use]
    pub fn routing(&self) -> Routing {
        self.view().routing()
    }

    /// **A file arrives in `layer`**, as the plugin's editor settles one: the layer's audio
    /// replaced, and every source-owned value the arrival decides written as a host gesture would
    /// leave it ([`import::Arrived::writes`]). The other layer is left alone.
    pub fn arrive(&mut self, layer: usize, imported: Imported) -> Result<(), String> {
        let arrival = imported.arrival();
        self.place(layer, imported.sample, arrival)
    }

    /// **A generated source arrives in `layer`**, as the plugin's picker puts one there.
    pub fn generate(&mut self, layer: usize, recipe: Recipe) -> Result<(), String> {
        self.place(layer, payload::encode_generated(recipe), Arrival::Generated)
    }

    fn place(
        &mut self,
        layer: usize,
        sample: EncodedSample,
        arrival: Arrival,
    ) -> Result<(), String> {
        if layer >= 2 {
            return Err("the destination layer does not exist".to_owned());
        }
        let playing = payload::prepare_sample(&sample)?;
        let arrived = import::resolve(arrival, &sample.name, Some(&playing));
        self.samples.layers[layer] = Some(sample);
        for (id, normalized) in arrived.writes(layer) {
            self.set_normalized(id, normalized);
        }
        Ok(())
    }

    /// Empties `layer`, as the plugin's chip cross does: no parameter moves.
    pub fn clear_layer(&mut self, layer: usize) -> Result<(), String> {
        let slot = self
            .samples
            .layers
            .get_mut(layer)
            .ok_or_else(|| "the destination layer does not exist".to_owned())?;
        *slot = None;
        Ok(())
    }
}

/// What a stored value restores to on `kind`, as nice-plug's `deserialize_object` applies it, or
/// `None` where nice-plug ignores it: a value of the wrong kind, or a choice it cannot name.
fn restored(kind: &Kind, value: &ParamValue) -> Option<f32> {
    match (kind, value) {
        (Kind::Float(_), ParamValue::F32(plain)) => Some(*plain),
        (Kind::Switch(_), ParamValue::Bool(on)) => Some(f32::from(u8::from(*on))),
        (Kind::Choice(choice), ParamValue::String(id)) => choice
            .variants
            .iter()
            .position(|variant| variant.id == id)
            .map(|index| index as f32),
        // An index past the variants is ignored here; the plugin would store it and fail to name it.
        (Kind::Choice(choice), ParamValue::I32(index)) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < choice.variants.len())
            .map(|index| index as f32),
        _ => None,
    }
}

fn serialize(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("a persisted field always serialises")
}

/// nice-plug's acceptance of a persisted field: it writes back the same text, or the same JSON.
fn kept_as_read(requested: &str, restored: &str) -> bool {
    requested == restored
        || serde_json::from_str::<serde_json::Value>(requested)
            .ok()
            .zip(serde_json::from_str::<serde_json::Value>(restored).ok())
            .is_some_and(|(requested, restored)| requested == restored)
}

/// **Every parameter's plain value, in [`params::all`] order, as the DSP reads them**, apart from a
/// state's audio: what the web tracker's AudioWorklet holds of a track, whose samples it keeps
/// apart. A [`State`] reads its own values through it, so the two can't disagree.
#[derive(Debug, Clone, Copy)]
pub struct Values<'a>(&'a [f32]);

impl<'a> Values<'a> {
    /// A view of `values`, none unless there is one for every parameter.
    #[must_use]
    pub fn new(values: &'a [f32]) -> Option<Self> {
        (values.len() == params::all().len()).then_some(Self(values))
    }

    /// `id`'s value as the host's `0..=1`.
    fn normalized(&self, id: &str) -> Option<f32> {
        let &index = INDEX.get(id)?;
        Some(params::all()[index].normalized(self.0[index]))
    }

    fn plain(&self, id: &str) -> f32 {
        self.0[INDEX[id]]
    }

    fn on(&self, id: &str) -> bool {
        self.plain(id) > 0.5
    }

    fn index(&self, id: &str) -> usize {
        self.plain(id).max(0.0) as usize
    }

    fn layer(&self, layer: usize) -> sound::Layer {
        let ids = LAYERS[layer];
        sound::Layer {
            reader: READERS.value(self.index(ids.reader)),
            root: self.plain(ids.root),
            transpose: self.plain(ids.transpose),
            start: self.plain(ids.start),
            end: self.plain(ids.end),
            loop_start: self.plain(ids.loop_start),
            loop_end: self.plain(ids.loop_end),
            loop_crossfade_s: self.plain(ids.loop_crossfade),
            loop_mode: LOOP_MODES.value(self.index(ids.loop_mode)),
            reverse: self.on(ids.reverse),
            level: self.plain(ids.level),
            pan: self.plain(ids.pan),
            scan_speed: self.plain(ids.scan_speed),
            grain_size_s: self.plain(ids.grain_size),
            grain_rate_hz: self.plain(ids.grain_rate),
            pitch_variation_st: self.plain(ids.pitch_variation),
            position_variation: self.plain(ids.position_variation),
            onset_timing: self.plain(ids.onset_timing),
            stereo_width: self.plain(ids.stereo_width),
            reverse_chance: self.plain(ids.reverse_chance),
            grain_shape: self.plain(ids.grain_shape),
        }
    }

    /// Every value the DSP reads but the routes.
    #[must_use]
    pub fn sound(&self) -> Sound {
        Sound {
            layers: [self.layer(0), self.layer(1)],
            character: sound::Character {
                sample_rate: self.plain(id::SAMPLE_RATE),
                bit_depth: self.plain(id::BIT_DEPTH),
                bit_smoothing: self.plain(id::BIT_SMOOTHING),
                converter_drive: self.plain(id::CONVERTER_DRIVE),
                converter_type: CONVERTER_TYPES.value(self.index(id::CONVERTER_TYPE)),
                jitter: self.plain(id::JITTER),
            },
            filter: sound::Filter {
                cutoff_hz: self.plain(id::CUTOFF),
                resonance: self.plain(id::RESONANCE),
                mode: FILTER_MODES.value(self.index(id::FILTER_MODE)),
                drive: self.plain(id::DRIVE),
            },
            envelope: sound::Envelope {
                attack_s: self.plain(id::ATTACK),
                decay_s: self.plain(id::DECAY),
                sustain: self.plain(id::SUSTAIN),
                release_s: self.plain(id::RELEASE),
            },
            lfos: [
                sound::Lfo {
                    rate_hz: self.plain(id::LFO1_RATE),
                    shape: LFO_SHAPES.value(self.index(id::LFO1_SHAPE)),
                },
                sound::Lfo {
                    rate_hz: self.plain(id::LFO2_RATE),
                    shape: LFO_SHAPES.value(self.index(id::LFO2_SHAPE)),
                },
            ],
            performance: sound::Performance {
                master: self.plain(id::MASTER),
                voices: VOICE_MODES.value(self.index(id::VOICE_MODE)),
                glide_s_per_octave: self.plain(id::GLIDE),
                bend_range_st: self.plain(id::BEND_RANGE),
            },
        }
    }

    /// **What the DSP plays for these values** at `tempo`, in beats a minute (`None` with no
    /// tempo): the plugin's own mapping, with each tempo sync resolved as the plugin resolves it
    /// once a callback.
    #[must_use]
    pub fn dsp_params(&self, tempo: Option<f64>) -> Params {
        let mut params = self.sound().dsp();
        let synced = TEMPO_SYNCS.map(|sync| {
            let rate = self
                .normalized(sync.rate)
                .expect("a sync's rate is a parameter");
            sync.resolve(self.on(sync.switch), tempo, rate)
        });
        if let Some(hz) = synced[0] {
            params.lfo_rate_hz = hz;
        }
        if let Some(hz) = synced[1] {
            params.lfo2_rate_hz = hz;
        }
        if let Some(hz) = synced[2] {
            params.layers[0].grain_density_hz = hz;
        }
        if let Some(hz) = synced[3] {
            params.layers[1].grain_density_hz = hz;
        }
        params
    }

    /// **The routing the DSP plays for these values**: each pair's presence, a present route's
    /// amount as stored, and the topology compacted.
    ///
    /// The plugin seeds every pair's amount from its normalised value once a callback and then
    /// writes each live one per sample from its smoother; settled, that is this, an absent pair's
    /// amount included, though nothing reads it.
    #[must_use]
    pub fn routing(&self) -> Routing {
        let mut routing = Routing::new();
        for (target, ids) in ROUTE_IDS.iter().enumerate() {
            for (source, &(amount, presence)) in ids.iter().enumerate() {
                let present = self.on(presence);
                routing.present[target][source] = present;
                routing.amounts[target][source] = if present {
                    self.plain(amount)
                } else {
                    let normalized = self.normalized(amount).expect("a route amount");
                    normalized * 2.0 - 1.0
                };
            }
        }
        routing.compact();
        routing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(state: &State) -> serde_json::Value {
        serde_json::from_slice(&state.to_json()).unwrap()
    }

    /// **A fresh state writes, reads and writes again equal**, through the JSON and the CLAP
    /// stream alike.
    #[test]
    fn a_fresh_state_writes_back_equal() {
        let state = State::new();
        let written = state.to_json();
        let read = State::from_json(&written).expect("its own JSON reads");
        assert_eq!(read, state);
        assert_eq!(read.to_json(), written);
        let stream = state.to_clap();
        assert_eq!(&stream[..8], &(written.len() as u64).to_le_bytes());
        assert_eq!(
            State::from_clap(&stream).expect("its own stream reads"),
            state
        );
    }

    /// **Bare values read as their state does**: the same DSP parameters and routing, so a player
    /// holding a track's values apart from its audio plays what the plugin plays.
    #[test]
    fn values_alone_play_what_their_state_plays() {
        let mut state = State::new();
        assert!(state.set_value(params::id::CUTOFF, 1_234.0));
        assert!(state.set_value(params::id::LFO1_SYNC, 1.0));
        let values = Values::new(state.values()).expect("a value for every parameter");
        let shown = |tempo| format!("{:?}", values.dsp_params(tempo));
        assert_eq!(
            shown(Some(97.0)),
            format!("{:?}", state.dsp_params(Some(97.0)))
        );
        assert_eq!(
            format!("{:?}", values.routing()),
            format!("{:?}", state.routing())
        );
        assert!(Values::new(&state.values()[1..]).is_none());
    }

    /// **A state reads as the plugin loads one**: what it leaves out keeps Init, a tempo sync left
    /// out is off, and a value of the wrong kind or for no parameter is ignored.
    #[test]
    fn a_state_reads_as_the_plugin_loads_one() {
        let mut edited = State::new();
        edited.set_normalized("alevel", 0.25);
        edited.set_normalized(id::LFO1_SYNC, 1.0);
        let mut value = json(&edited);
        let params = value["params"].as_object_mut().unwrap();
        params.remove(id::LFO1_SYNC);
        params.remove("cutoff");
        params.insert("blend".to_owned(), serde_json::json!({ "f32": 0.75 }));
        params.insert("resonance".to_owned(), serde_json::json!({ "bool": true }));
        params.insert("areader".to_owned(), serde_json::json!({ "i32": 1 }));
        let read = State::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(read.value("alevel"), Some(0.25));
        assert_eq!(
            read.value(id::LFO1_SYNC),
            Some(0.0),
            "a sync left out is off"
        );
        assert_eq!(read.value("cutoff"), State::new().value("cutoff"));
        assert_eq!(read.value("resonance"), State::new().value("resonance"));
        assert_eq!(read.text("areader").as_deref(), Some("Stretch"));
    }

    /// **What the plugin refuses whole is an error**: malformed audio, an unknown field, a preset
    /// that is no identity.
    #[test]
    fn what_the_plugin_refuses_is_an_error() {
        let fresh = json(&State::new());
        let refused = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut value = fresh.clone();
            edit(&mut value);
            State::from_json(&serde_json::to_vec(&value).unwrap()).unwrap_err()
        };
        assert!(matches!(
            refused(&|v| v["fields"]["samples"] = "{ definitely not JSON".into()),
            StateError::Samples(_)
        ));
        assert!(matches!(
            refused(&|v| v["fields"]["notes"] = "{}".into()),
            StateError::UnknownField(_)
        ));
        assert!(matches!(
            refused(&|v| v["fields"]["preset"] = r#"{"version": "one"}"#.into()),
            StateError::Preset(_)
        ));
        // Read as a sequence, every field its default, and so not written back as it was asked.
        assert!(matches!(
            refused(&|v| v["fields"]["preset"] = "[]".into()),
            StateError::NotKept(_)
        ));
        assert!(State::from_clap(&[1, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    /// **The text a host shows is read back by the same row**, for every parameter of a fresh
    /// state.
    #[test]
    fn every_row_reads_back_its_own_text() {
        let mut state = State::new();
        for param in params::all() {
            let text = state.text(param.id).unwrap();
            assert!(state.type_text(param.id, &text), "{}: {text:?}", param.id);
            assert_eq!(state.text(param.id).unwrap(), text, "{}", param.id);
        }
    }

    /// **A generated source arrives looping over its whole buffer, in tune by its name.**
    #[test]
    fn a_generated_source_arrives_looping_and_in_tune() {
        let mut state = State::new();
        state.set_normalized("bloop", 0.0);
        state.generate(1, Recipe::Glass).unwrap();
        assert_eq!(state.text("bloop").as_deref(), Some("Forward"));
        assert!((state.value("broot").unwrap() - crate::generate::ROOT).abs() < 1.0e-3);
        assert!(state.samples().layers[1].is_some());
        assert!(state.generate(2, Recipe::Glass).is_err());
    }
}
