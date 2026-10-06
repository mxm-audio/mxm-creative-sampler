//! The seven brief-owned cards.

use crate::asset::LoadState;
use crate::binding::{Bound, segmented_named, segmented_waves_named};
use crate::params::MxmCreativeSamplerParams;
use egui::{Pos2, Rect, Sense, Stroke, Ui, Vec2};
use mxm_creative_sampler_dsp::Sample;
use mxm_ui::control::{Size, Wave};
use mxm_ui::space::{MIN_TARGET, SPACE_3};
use mxm_ui::theme::Tokens;
use mxm_ui::tree::{
    self, Flow, Font, Height, Kind, Node, Share, leaf, pad, pad_all, row_gap, share, stack,
};
use nice_plug::prelude::ParamSetter;
use std::collections::HashMap;
use std::path::PathBuf;

const KNOB: Size = Size::Standard;

pub fn all_parameters(params: &MxmCreativeSamplerParams) -> Vec<Bound<'_>> {
    vec![
        Bound::new(
            "aroot",
            &params.a_root,
            "Sets the keyboard note that plays A at its recorded rate.",
        )
        .stepped(ROOT_NOTE_STEP, ROOT_OCTAVE_STEP),
        Bound::new(
            "atranspose",
            &params.a_transpose,
            "Transposes layer A.",
        )
        .bipolar(),
        Bound::new(
            "astart",
            &params.a_start,
            "Where layer A starts playing in the sample.",
        ),
        Bound::new(
            "aend",
            &params.a_end,
            "Where layer A stops playing in the sample.",
        ),
        Bound::new(
            "aloopstart",
            &params.a_loop_start,
            "Where A's loop starts.",
        ),
        Bound::new(
            "aloopend",
            &params.a_loop_end,
            "Where A's loop ends.",
        ),
        Bound::new(
            "aloopcrossfade",
            &params.a_loop_crossfade,
            "Blends A's loop end into its start, so the loop does not click.",
        ),
        Bound::new("alevel", &params.a_level, "Sets how loud layer A is."),
        Bound::new("apan", &params.a_pan, "Places layer A in the stereo field.").bipolar(),
        Bound::new(
            "ascanspeed",
            &params.a_scan_speed,
            "How fast playback moves through the sample in Stretch and Grain; zero stands still, below zero goes backwards.",
        )
        .bipolar(),
        Bound::new(
            "agrainsize",
            &params.a_grain_size,
            "How long A's grains last.",
        ),
        Bound::new(
            "agrainrate",
            &params.a_grain_rate,
            "How many A grains start each second.",
        ),
        Bound::new(
            "agrainsync",
            &params.a_grain_sync,
            crate::binding::SYNC_DESCRIPTION,
        ),
        Bound::new(
            "apitchvar",
            &params.a_pitch_var,
            "Detunes each of A's grains by up to this much, for a thicker sound.",
        ),
        Bound::new(
            "atiming",
            &params.a_timing,
            "How irregularly A's grains start: even, then uneven, then a cloud.",
        ),
        Bound::new(
            "areversechance",
            &params.a_reverse_chance,
            "How often one of A's grains reads backwards.",
        ),
        Bound::new(
            "agrainshape",
            &params.a_grain_shape,
            "The shape of each grain: hard and clicky, a triangle, or smooth; harder is brighter.",
        ),
        Bound::new(
            "aposvar",
            &params.a_position_var,
            "Takes each of A's grains from a different part of the sample, for a thick cloud.",
        ),
        Bound::new(
            "awidth",
            &params.a_width,
            "Spreads A's grains across the stereo field.",
        ),
        Bound::new(
            "broot",
            &params.b_root,
            "Sets the keyboard note that plays B at its recorded rate.",
        )
        .stepped(ROOT_NOTE_STEP, ROOT_OCTAVE_STEP),
        Bound::new(
            "btranspose",
            &params.b_transpose,
            "Transposes layer B.",
        )
        .bipolar(),
        Bound::new(
            "bstart",
            &params.b_start,
            "Where layer B starts playing in the sample.",
        ),
        Bound::new(
            "bend",
            &params.b_end,
            "Where layer B stops playing in the sample.",
        ),
        Bound::new(
            "bloopstart",
            &params.b_loop_start,
            "Where B's loop starts.",
        ),
        Bound::new(
            "bloopend",
            &params.b_loop_end,
            "Where B's loop ends.",
        ),
        Bound::new(
            "bloopcrossfade",
            &params.b_loop_crossfade,
            "Blends B's loop end into its start, so the loop does not click.",
        ),
        Bound::new("blevel", &params.b_level, "Sets how loud layer B is."),
        Bound::new("bpan", &params.b_pan, "Places layer B in the stereo field.").bipolar(),
        Bound::new(
            "bscanspeed",
            &params.b_scan_speed,
            "How fast playback moves through the sample in Stretch and Grain; zero stands still, below zero goes backwards.",
        )
        .bipolar(),
        Bound::new(
            "bgrainsize",
            &params.b_grain_size,
            "How long B's grains last.",
        ),
        Bound::new(
            "bgrainrate",
            &params.b_grain_rate,
            "How many B grains start each second.",
        ),
        Bound::new(
            "bgrainsync",
            &params.b_grain_sync,
            crate::binding::SYNC_DESCRIPTION,
        ),
        Bound::new(
            "bpitchvar",
            &params.b_pitch_var,
            "Detunes each of B's grains by up to this much, for a thicker sound.",
        ),
        Bound::new(
            "btiming",
            &params.b_timing,
            "How irregularly B's grains start: even, then uneven, then a cloud.",
        ),
        Bound::new(
            "breversechance",
            &params.b_reverse_chance,
            "How often one of B's grains reads backwards.",
        ),
        Bound::new(
            "bgrainshape",
            &params.b_grain_shape,
            "The shape of each grain: hard and clicky, a triangle, or smooth; harder is brighter.",
        ),
        Bound::new(
            "bposvar",
            &params.b_position_var,
            "Takes each of B's grains from a different part of the sample, for a thick cloud.",
        ),
        Bound::new(
            "bwidth",
            &params.b_width,
            "Spreads B's grains across the stereo field.",
        ),
        Bound::new(
            // **Four descriptions rewritten to say what you hear.** They described the
            // implementation — "the virtual source rate before reconstruction", "an analytic
            // companding curve" — which is exactly the vocabulary a player cannot check against
            // the sound. Design system §7.1 asks for one sentence on what it does.
            "rate",
            &params.rate,
            "Lowers the sample rate, like an old sampler: the top of the sound goes and grit folds back in.",
        ),
        Bound::new(
            "converter",
            &params.converter,
            "The bit depth: fewer bits add grainy noise, most audible in quiet passages.",
        ),
        Bound::new(
            "reconstruction",
            &params.reconstruction,
            "Smooth between samples, or stepped for a bright buzz.",
        ),
        Bound::new(
            "input",
            &params.input,
            "Drives the sound harder into the old-sampler stage: saturated, with more of its grit.",
        ),
        Bound::new(
            "convtype",
            &params.converter_type,
            "How the bit depth treats quiet sounds.",
        ),
        Bound::new(
            "jitter",
            &params.jitter,
            "Makes the old-sampler timing unsteady, from a shimmer to a lurch.",
        ),
        Bound::new(
            "cutoff",
            &params.cutoff,
            "The filter's cutoff: lower is darker.",
        )
        .law(mxm_preset::StepLaw::Hertz),
        Bound::new(
            "resonance",
            &params.resonance,
            "Emphasis at the cutoff.",
        ),
        Bound::new(
            "drive",
            &params.drive,
            "Drives the filter harder, for saturation.",
        ),
        Bound::new(
            "attack",
            &params.attack,
            "How long a note takes to reach full level.",
        ),
        Bound::new(
            "decay",
            &params.decay,
            "How long a note takes to fall to the sustain level.",
        ),
        Bound::new(
            "sustain",
            &params.sustain,
            "The level held while a key is down.",
        ),
        Bound::new(
            "release",
            &params.release,
            "How long a note takes to fade after the key is released.",
        ),
        Bound::new(
            "lforate",
            &params.lfo_rate,
            "How fast LFO 1 runs, from a slow drift to a fast flutter.",
        ),
        Bound::new(
            "lfo1sync",
            &params.lfo1_sync,
            crate::binding::SYNC_DESCRIPTION,
        ),
        Bound::new(
            "lfo1shape",
            &params.lfo_shape,
            "LFO 1's shape.",
        ),
        Bound::new(
            "lfo2rate",
            &params.lfo2_rate,
            "How fast LFO 2 runs, from a slow drift to a fast flutter.",
        ),
        Bound::new(
            "lfo2sync",
            &params.lfo2_sync,
            crate::binding::SYNC_DESCRIPTION,
        ),
        Bound::new(
            "lfo2shape",
            &params.lfo2_shape,
            "LFO 2's shape.",
        ),
        Bound::new(
            "master",
            &params.master,
            "Sets the instrument's final level.",
        ),
        Bound::new(
            "glide",
            &params.glide,
            "Sets how long a Mono or Legato glide takes to cover one octave; zero is off.",
        ),
        Bound::new(
            "bendrange",
            &params.bend_range,
            "Sets how far the pitch wheel moves a sounding note.",
        )
        .law(mxm_preset::StepLaw::Semitones),
    ]
}

/// A card's binding: painted as `binding::layer_label` says — without the layer the layer bar
/// already names, `Converter`'s heading, or the module its card is titled by. The host name keeps
/// all of it.
fn bound<'a>(id: &str, params: &'a MxmCreativeSamplerParams) -> Bound<'a> {
    let binding = all_parameters(params)
        .into_iter()
        .find(|binding| binding.id == id)
        .unwrap_or_else(|| panic!("no binding for {id}"));
    let label = crate::binding::layer_label(binding.param.name());
    binding.labelled(label)
}

/// A target's **painted** name: without the layer the layer bar already names — *Scan speed* over
/// the selected layer's, not *Scan speed A*. `TARGET_NAMES` stays the canonical name.
fn panel_name(target: usize) -> &'static str {
    let name = mxm_creative_sampler_dsp::routing::TARGET_NAMES[target];
    name.strip_suffix(" A")
        .or_else(|| name.strip_suffix(" B"))
        .unwrap_or(name)
}

/// The cards, in paging order.
pub(crate) const TITLES: [&str; 7] = [
    "Keyboard",
    "Envelope",
    // **An LFO module, not a Travel module.** It was one LFO wired to one destination, and the card
    // carried that destination's controls; both LFOs are ordinary sources now, so where they reach
    // is drawn under the thing they reach rather than here.
    "LFOs",
    "Sample",
    "Playback",
    "Converter",
    "Filter",
];

/// What the cards show besides the parameters: the layer selected in the layer bar, which layers
/// hold a sample, their names and the acquisition's state. A card's shape follows each of them —
/// an empty layer has no region, level or Root rows, a name sets its chip's width, a status line
/// wraps or truncates — so they are inputs to the trees, read once per frame.
pub struct Shown<'a> {
    pub layer: usize,
    pub loaded: [bool; 2],
    pub names: [&'a str; 2],
    pub status: &'a LoadState,
}

/// Every paging item, each floor computed from its card's tree in `ui`'s fonts. Ceilings stay
/// [`CARD_CEILING`].
pub fn page_items(
    ui: &Ui,
    params: &MxmCreativeSamplerParams,
    shown: &Shown<'_>,
) -> Vec<mxm_ui::paging::Item<'static>> {
    use mxm_ui::flow::Card;
    use mxm_ui::paging::{Category, Item, Key};
    const CATEGORIES: [Category; 7] = [
        Category::Performance,
        Category::Modulators,
        Category::Modulators,
        Category::Generators,
        Category::Generators,
        Category::Generators,
        Category::Tone,
    ];
    TITLES
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let floor = tree::card_floor(ui, title, &card(ui, index, params, shown));
            (index, title, floor)
        })
        .map(|(index, title, floor)| Item {
            key: Key(index as u64),
            // Exactly as wide as its content: the ceiling is the floor
            // (`plans/plan-editor-standard.md` A1).
            card: Card::new(title, floor).capped(floor),
            category: CATEGORIES[index],
            kind: title,
        })
        .collect()
}

/// The paging items as the editor computes them for `shown`, from a context set up as an editor's
/// is — three passes in, so the weighted font cuts are bound — for tests, which have no editor `Ui`
/// to hand.
#[cfg(test)]
pub(crate) fn test_items(
    params: &MxmCreativeSamplerParams,
    shown: &Shown<'_>,
) -> Vec<mxm_ui::paging::Item<'static>> {
    let ctx = egui::Context::default();
    mxm_ui::typography::apply(&ctx);
    mxm_ui::theme::apply(&ctx);
    let mut items = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            items = page_items(ui, params, shown);
        });
        output.textures_delta.clear();
    }
    items
}

#[allow(clippy::too_many_arguments)]
pub fn cards(
    ui: &mut Ui,
    tokens: &Tokens,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    text: &mut HashMap<&'static str, Option<String>>,
    selected_layer: &mut usize,
    samples: [Option<&Sample>; 2],
    names: [&str; 2],
    status: &LoadState,
    waveform: &[[f32; 2]],
    playhead: Option<f32>,
    telemetry: &crate::telemetry::Telemetry,
    load: &mut dyn FnMut(usize, PathBuf),
    clear: &mut dyn FnMut(usize),
    generate: &mut dyn FnMut(usize, crate::generate::Recipe),
) {
    // Every card is built from the layer selected when the frame began; a chip clicked this frame
    // selects its layer on the next, when every tree is rebuilt around it.
    let shown = Shown {
        layer: *selected_layer,
        loaded: samples.map(|sample| sample.is_some()),
        names,
        status,
    };
    let items = page_items(ui, params, &shown);
    let text_editing = text.values().any(Option::is_some);
    let mut live = Live {
        params,
        setter,
        text,
        layer: shown.layer,
        chosen: None,
        samples,
        names,
        status,
        waveform,
        playhead,
        telemetry: Some(telemetry),
        load,
        clear,
        generate,
    };
    mxm_ui::paging::editor::show(
        ui,
        tokens,
        &items,
        &[],
        text_editing,
        &mut |ui, index| card(ui, index, params, &shown),
        &mut |ui, _, leaf, rect| paint(ui, tokens, leaf, rect, &mut live),
    );
    if let Some(layer) = live.chosen {
        *selected_layer = layer;
    }
}

// ---------------------------------------------------------------------------------------------
// The cards, as trees (plans/plan-layout-tree.md). Each card is described once — `card` — and that
// one description is both measured (its floor and its height) and drawn, leaf by leaf, through the
// bindings (`paint`). Nothing is typed and nothing is drawn to learn a size.
// ---------------------------------------------------------------------------------------------

/// A stepped parameter drawn as a segmented control.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Switch {
    Voices,
    Reader(usize),
    Loop(usize),
    Direction(usize),
    ConverterType,
    FilterMode,
}

// Stacked on the Playback card, so the three share one cell width and read as one set.
const READER: [&str; 3] = ["Repitch", "Stretch", "Grain"];
const LOOP: [&str; 3] = ["One shot", "Forward", "Ping-pong"];
const REVERSE: [&str; 2] = ["Forward", "Reverse"];

impl Switch {
    fn id(self) -> &'static str {
        match self {
            Self::Voices => "voicemode",
            Self::Reader(layer) => ["areader", "breader"][layer],
            Self::Loop(layer) => ["aloop", "bloop"][layer],
            Self::Direction(layer) => ["areverse", "breverse"][layer],
            Self::ConverterType => "convtype",
            Self::FilterMode => "filtermode",
        }
    }

    fn param(self, params: &MxmCreativeSamplerParams) -> &dyn mxm_preset::ErasedParam {
        match self {
            Self::Voices => &params.voice_mode,
            Self::Reader(0) => &params.a_reader,
            Self::Reader(_) => &params.b_reader,
            Self::Loop(0) => &params.a_loop,
            Self::Loop(_) => &params.b_loop,
            Self::Direction(0) => &params.a_reverse,
            Self::Direction(_) => &params.b_reverse,
            Self::ConverterType => &params.converter_type,
            Self::FilterMode => &params.filter_mode,
        }
    }

    fn options(self) -> &'static [&'static str] {
        match self {
            Self::Voices => &["Poly", "Mono", "Legato"],
            Self::Reader(_) => &READER,
            Self::Loop(_) => &LOOP,
            Self::Direction(_) => &REVERSE,
            Self::ConverterType => &["Linear", "Companding"],
            Self::FilterMode => &["Off", "Low-pass", "Band-pass", "High-pass"],
        }
    }

    /// What each option does, one sentence per cell (design system §7.3; the owner, 2026-09-27:
    /// the cells of a row do not share one sentence).
    fn details(self) -> &'static [&'static str] {
        match self {
            Self::Voices => &[
                "Up to eight notes at once.",
                "One note at a time; every key restarts the sound.",
                "One note at a time; keys played over a held one change its pitch without restarting.",
            ],
            Self::Reader(_) => &[
                "Higher notes play the sample faster and shorter, like tape.",
                "Pitch and length are separate: every note keeps the sample's length.",
                "Plays the sample as a cloud of tiny overlapping grains.",
            ],
            Self::Loop(_) => &[
                "Plays through once and stops.",
                "Loops over and over in one direction.",
                "Loops back and forth.",
            ],
            Self::Direction(_) => &["Plays the sample forwards.", "Plays the sample backwards."],
            Self::ConverterType => &[
                "Even steps: the harsher grit of a plain low-resolution sampler.",
                "Finer steps for quiet sounds: the smoother grit of vintage samplers.",
            ],
            Self::FilterMode => &[
                "No filter.",
                "Removes the highs: darker.",
                "Keeps a band around the cutoff: thin and nasal.",
                "Removes the lows: thinner and brighter.",
            ],
        }
    }
}

/// What a leaf of this editor's cards draws. Hashed by what it names — a parameter, a layer, a
/// target — which is also what keeps its widget ids stable when a route appears above it.
#[derive(Clone, Debug, Hash)]
pub(crate) enum Leaf {
    Knob(&'static str),
    /// A fader in the collection's fader row.
    Fader(&'static str),
    Switch(Switch),
    /// An LFO's shapes, beside its rate knob.
    Waves(&'static str),
    /// A control's tempo sync, the quarter note beside it.
    Picture(&'static str),
    Routes(usize),
    /// The amplitude envelope, drawn.
    Envelope,
    /// The layer bar: a layer's chip and its cross, and Load.
    Chip(usize),
    Remove(usize),
    Load,
    Generate,
    /// The selected layer's waveform: the region editor, the drop target and the grains.
    Waveform,
    Status,
    /// Root's octave buttons (up or not), Auto, and what Auto heard.
    Octave(usize, bool),
    Auto(usize),
    Heard(String),
    /// A layer's grain envelope and its overlap reading.
    GrainEnvelope(usize),
    Overlap(usize),
}

/// A layer's parameter id: `suffix` on `a` or `b`, as [`all_parameters`] names it.
fn layer_id(params: &MxmCreativeSamplerParams, layer: usize, suffix: &str) -> &'static str {
    bound(
        &format!("{}{suffix}", if layer == 0 { 'a' } else { 'b' }),
        params,
    )
    .id
}

/// A knob standing in `column` — `0.0` in the collection's knob row, which sizes the columns, or
/// [`KNOB_COLUMN_MIN`](mxm_ui::control::KNOB_COLUMN_MIN) standing alone — its name without the layer
/// the bar already says.
fn knob_leaf(params: &MxmCreativeSamplerParams, id: &str, column: f32) -> Node<Leaf> {
    let bound = bound(id, params);
    // A syncable control's column holds its free readings and its divisions.
    let widest = match ladder_of(bound.id) {
        Some(ladder) => crate::binding::synced_widest(bound.param, ladder.span),
        None => mxm_ui::control::widest_value(|n| bound.param.format(n as f32)),
    };
    leaf(
        Leaf::Knob(bound.id),
        Kind::Knob {
            name: crate::binding::layer_label(bound.param.name()),
            widest,
            size: KNOB,
            column,
        },
    )
}

/// The ladder of the control `id`, if it has a tempo sync (`plans/plan-tempo-sync-controls.md`).
fn ladder_of(id: &str) -> Option<mxm_tempo::Ladder> {
    match id {
        "lforate" | "lfo2rate" => Some(crate::params::LFO_SYNC),
        "agrainrate" | "bgrainrate" => Some(crate::params::GRAIN_SYNC),
        _ => None,
    }
}

/// A control's tempo sync, the quarter note on the grid of the knob beside it.
fn sync_beside(params: &MxmCreativeSamplerParams, id: &str) -> Node<Leaf> {
    mxm_ui::tree::switch_beside_knob(
        KNOB,
        leaf(Leaf::Picture(bound(id, params).id), Kind::SyncToggle),
    )
}

/// What an envelope fader paints: its stage's letter, by the convention (the owner, 2026-09-25). A
/// host, the tooltip and a screen reader read *Attack*.
fn fader_label(id: &str) -> Option<&'static str> {
    match id {
        "attack" => Some("A"),
        "decay" => Some("D"),
        "sustain" => Some("S"),
        "release" => Some("R"),
        _ => None,
    }
}

/// The envelope's A, D, S and R as the collection's fader row (`tree::fader_row`).
fn fader_row(ui: &Ui, params: &MxmCreativeSamplerParams, ids: &[&str]) -> Node<Leaf> {
    mxm_ui::tree::fader_row(
        ui,
        ids.iter()
            .map(|id| {
                let bound = bound(id, params);
                mxm_ui::tree::fader(
                    Leaf::Fader(bound.id),
                    fader_label(bound.id).unwrap_or("?"),
                    mxm_ui::control::widest_value(|n| bound.param.format(n as f32)),
                )
            })
            .collect(),
    )
}

/// The collection's knob row (`mxm_ui::tree::knob_row`). Its columns hold each knob's widest reading
/// whole (`control::knob_size`): a narrower column truncated a one-line value, which is why Glide
/// and Cutoff once arrived as `0.060 s/o…` and `18000…`.
fn knob_row(ui: &Ui, params: &MxmCreativeSamplerParams, ids: &[&str]) -> Node<Leaf> {
    mxm_ui::tree::knob_row(
        ui,
        ids.iter()
            .map(|id| (KNOB, knob_leaf(params, id, 0.0)))
            .collect(),
    )
}

fn switch_leaf(params: &MxmCreativeSamplerParams, switch: Switch) -> Node<Leaf> {
    leaf(
        Leaf::Switch(switch),
        Kind::Segmented {
            label: crate::binding::layer_label(switch.param(params).name()),
            options: switch.options().iter().map(|o| (*o).to_owned()).collect(),
            beside: None,
        },
    )
}

/// A target's route stack, `SPACE_3` below what precedes it. **Routing belongs under the thing it
/// affects**, never in a detached footer. A stack is a composite with a rule of its own — its floor
/// is every route revealed — so it states its size (`stack_size`) and fills the card's width.
fn routes_leaf(ui: &Ui, params: &MxmCreativeSamplerParams, target: usize) -> Node<Leaf> {
    let size = mxm_modulation_params::ui::stack_size(
        ui,
        panel_name(target),
        &params.routes.target(target),
    );
    pad(
        SPACE_3,
        leaf(
            Leaf::Routes(target),
            Kind::Custom {
                min_width: size.x,
                height: Height::Fixed(size.y),
                fills: true,
            },
        ),
    )
}

/// A display that fills its card's width at a stated height.
fn display(key: Leaf, min_width: f32, height: f32) -> Node<Leaf> {
    leaf(
        key,
        Kind::Custom {
            min_width,
            height: Height::Fixed(height),
            fills: true,
        },
    )
}

/// Card `index`'s body, as a tree.
pub(crate) fn card(
    ui: &Ui,
    index: usize,
    params: &MxmCreativeSamplerParams,
    shown: &Shown<'_>,
) -> Node<Leaf> {
    use mxm_creative_sampler_dsp::routing::target;
    match index {
        0 => stack(vec![
            switch_leaf(params, Switch::Voices),
            knob_row(ui, params, &["glide", "bendrange"]),
            // **Pitch's stack, on the keyboard card.** It is the target a player reaches for from
            // here — vibrato from an LFO, a per-note detune from Random — and it sits beside Bend
            // range, the other control that says how far a gesture moves the pitch.
            routes_leaf(ui, params, target::PITCH),
        ]),
        1 => stack(vec![
            display(Leaf::Envelope, 0.0, ENVELOPE_HEIGHT),
            fader_row(ui, params, &["attack", "decay", "sustain", "release"]),
            // **Amplitude's stack, under the envelope that owns the amplifier.** This target's law
            // is multiplicative, so a route here scales the envelope instead of adding to it.
            routes_leaf(ui, params, target::AMPLITUDE),
        ]),
        // **Two LFOs and nothing else**, which is what the owner asked for: a route stack belongs
        // under the control it moves, so Scan speed's stacks are on the Playback card.
        2 => stack(vec![
            lfo_row(ui, params, "lforate", "lfo1sync", "lfo1shape"),
            lfo_row(ui, params, "lfo2rate", "lfo2sync", "lfo2shape"),
        ]),
        3 => sample(ui, params, shown),
        4 => playback(ui, params, shown.layer),
        // **This is not an advanced corner, it is the reason the instrument exists**: the four
        // stages are the card. **Jitter sits on its own line**, under the stages whose clock it
        // unsteadies: it is *when* they sample rather than what they do. **No prose under the
        // knobs** (the owner's ruling): the facts live in `AGENTS.md` and
        // `docs/oscillators/14-samplers.md` §14.6.
        5 => stack(vec![
            switch_leaf(params, Switch::ConverterType),
            knob_row(
                ui,
                params,
                &["rate", "converter", "reconstruction", "input"],
            ),
            knob_row(ui, params, &["jitter"]),
        ]),
        // One row: three knobs fit the card's width. **Cutoff's stack, under the filter**; what
        // opens it besides the knob is Cutoff's tooltip (the owner, 2026-09-27: no help text on the
        // panel).
        _ => stack(vec![
            switch_leaf(params, Switch::FilterMode),
            knob_row(ui, params, &["cutoff", "resonance", "drive"]),
            routes_leaf(ui, params, target::CUTOFF),
        ]),
    }
}

const LOAD: &str = "Load";
const GENERATE: &str = "Generate";
const OCT_DOWN: &str = "Oct −";
const OCT_UP: &str = "Oct +";
const AUTO: &str = "Auto";
/// The Envelope card's drawing is this tall and fills the card's width.
const ENVELOPE_HEIGHT: f32 = 72.0;
/// The Sample card's waveform is this tall and fills the card's width.
const WAVEFORM_HEIGHT: f32 = 154.0;

/// One LFO: its rate knob, and its shapes beside it on the knob's grid, `SPACE_3` past the row's
/// spacing.
fn lfo_row(
    ui: &Ui,
    params: &MxmCreativeSamplerParams,
    rate: &str,
    sync: &str,
    shape: &str,
) -> Node<Leaf> {
    let shape = bound(shape, params);
    row_gap(
        ui.spacing().item_spacing.x,
        vec![
            knob_leaf(params, rate, mxm_ui::control::KNOB_COLUMN_MIN),
            // The rate's tempo sync, then its shapes.
            sync_beside(params, sync),
            pad_all(
                0.0,
                SPACE_3,
                0.0,
                leaf(
                    Leaf::Waves(shape.id),
                    Kind::Waves {
                        label: Some(crate::binding::layer_label(shape.param.name())),
                        count: LFO_SHAPES.len(),
                        marks: Vec::new(),
                        beside: Some(KNOB),
                    },
                ),
            ),
        ],
    )
}

/// The Sample card: the layer bar, the generated sources, the waveform, the acquisition's status
/// and — when the selected layer holds a sample — its region, its level and Root, and Root's
/// helpers.
///
/// **The layer bar is persistent context, not a prefix on every control**: the selected layer is
/// named once there and every control below belongs to it. Its row is one height, `MIN_TARGET`:
/// the chips, the crosses and Load all stand at the pointer target.
fn sample(ui: &Ui, params: &MxmCreativeSamplerParams, shown: &Shown<'_>) -> Node<Leaf> {
    let gap = ui.spacing().item_spacing.x;
    let chip = |layer: usize| {
        leaf(
            Leaf::Chip(layer),
            Kind::Button {
                label: chip_label(shown.names[layer]),
                min: Vec2::new(CHIP_WIDTH, MIN_TARGET),
                fills: false,
            },
        )
    };
    // The square is allocated whether or not it draws, so the row does not reflow as layers load.
    let mark = |layer: usize| leaf(Leaf::Remove(layer), Kind::RemoveMark { size: MIN_TARGET });
    let (status, flow) = status_line(shown.status);
    let mut body = vec![
        row_gap(
            gap,
            vec![
                chip(0),
                mark(0),
                chip(1),
                mark(1),
                leaf(
                    Leaf::Load,
                    Kind::Button {
                        label: LOAD.to_owned(),
                        min: Vec2::new(0.0, MIN_TARGET),
                        fills: false,
                    },
                ),
            ],
        ),
        leaf(
            Leaf::Generate,
            Kind::Selector {
                label: GENERATE.to_owned(),
                options: generated_labels().map(str::to_owned).to_vec(),
                caption: false,
                width: None,
            },
        ),
        display(Leaf::Waveform, 0.0, WAVEFORM_HEIGHT),
        leaf(
            Leaf::Status,
            Kind::Text {
                text: status,
                font: Font::Body,
                flow,
            },
        ),
    ];
    if shown.loaded[shown.layer] {
        let layer = shown.layer;
        let ids = |suffixes: &[&str]| -> Vec<&'static str> {
            suffixes
                .iter()
                .map(|suffix| layer_id(params, layer, suffix))
                .collect()
        };
        // One row: the whole region reads left to right, in the order it plays, and ends on the
        // fade that joins the loop back to itself. **Level and pan belong to the layer, so they live
        // where the layer is chosen**, and **Root follows Pan** (owner, 2026-09-13).
        body.push(knob_row(
            ui,
            params,
            &ids(&["start", "end", "loopstart", "loopend", "loopcrossfade"]),
        ));
        body.push(knob_row(ui, params, &ids(&["level", "pan", "root"])));
        // **The octave buttons and Auto move with Root, under the row that holds it.**
        body.push(root_helpers(ui, params, layer));
    }
    stack(body)
}

/// What a layer chip says: its file name shortened from the middle, or how to fill it.
fn chip_label(name: &str) -> String {
    if name.is_empty() {
        "empty — drop a WAV".to_owned()
    } else {
        elided(name, CHIP_NAME_BUDGET)
    }
}

/// The status line under the waveform, and how it uses its width: the resting line wraps, and a
/// line that carries a file name truncates — a source name is the one string this instrument does
/// not choose, and it must never push its card wider.
fn status_line(status: &LoadState) -> (String, Flow) {
    let letter = |layer: usize| if layer == 0 { "A" } else { "B" };
    match status {
        LoadState::Idle => (
            "WAV • audio is embedded when the host saves state".to_owned(),
            Flow::Wrap,
        ),
        LoadState::Loading { layer, name } => (
            format!("Loading {name} into {}…", letter(*layer)),
            Flow::Truncate,
        ),
        LoadState::Ready { layer, name } => (
            format!("{name} is ready in {}", letter(*layer)),
            Flow::Truncate,
        ),
        // A decode failure names the file that failed, so this line carries the same overflow risk
        // the chips did.
        LoadState::Failed { layer, message } => (
            format!("{} was kept: {message}", letter(*layer)),
            Flow::Truncate,
        ),
    }
}

/// Where Auto remembers what it heard on `layer`.
pub(crate) fn heard_id(layer: usize) -> egui::Id {
    egui::Id::new(("mxm-auto-root", layer))
}

/// What Auto last said about the sample `layer` holds now, if anything. **What it heard is about one
/// sample, so it is remembered with that sample's revision**: a drop, a Browse, a preset load and a
/// host state restore all retire the note alike.
fn heard(ctx: &egui::Context, params: &MxmCreativeSamplerParams, layer: usize) -> Option<String> {
    ctx.data(|data| data.get_temp::<(u64, String)>(heard_id(layer)))
        .filter(|(on, _)| *on == params.assets.revision())
        .map(|(_, said)| said)
}

/// Root's helpers: the octave shift, first because it is the one that gets used, then Auto, then
/// what Auto heard — in a row whose items stand centred on its line, as `horizontal` put them.
fn root_helpers(ui: &Ui, params: &MxmCreativeSamplerParams, layer: usize) -> Node<Leaf> {
    let button = |key: Leaf, label: &str| {
        leaf(
            key,
            Kind::Button {
                label: label.to_owned(),
                min: Vec2::ZERO,
                fills: false,
            },
        )
    };
    let mut row = vec![
        button(Leaf::Octave(layer, false), OCT_DOWN),
        button(Leaf::Octave(layer, true), OCT_UP),
        button(Leaf::Auto(layer), AUTO),
    ];
    if let Some(said) = heard(ui.ctx(), params, layer) {
        let line = mxm_ui::control::button_size(ui, AUTO, Vec2::ZERO).y;
        let text = ui
            .painter()
            .layout_no_wrap(
                said.clone(),
                tree::font_id(ui, Font::Caption),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .y;
        row.push(pad(
            ((line - text) / 2.0).max(0.0),
            leaf(
                Leaf::Heard(said.clone()),
                Kind::Text {
                    text: said,
                    font: Font::Caption,
                    flow: Flow::Line,
                },
            ),
        ));
    }
    row_gap(ui.spacing().item_spacing.x, row)
}

/// Which reader `layer` is in: 0 Repitch, 1 Stretch, 2 Grain.
fn reader_mode(params: &MxmCreativeSamplerParams, layer: usize) -> usize {
    let mode = Switch::Reader(layer).param(params).normalised();
    if mode > 0.75 {
        2
    } else if mode > 0.25 {
        1
    } else {
        0
    }
}

/// The Playback card: the layer's three switches at one cell width, then **a mode's controls, which
/// belong to that mode** — Grain size and rate while Repitch is selected are noise, and their
/// absence is explained by the switch directly above. The card reserves the tallest mode, so the
/// rows below do not jump when a mode is chosen.
fn playback(ui: &Ui, params: &MxmCreativeSamplerParams, layer: usize) -> Node<Leaf> {
    let current = reader_mode(params, layer);
    let mut modes: Vec<Node<Leaf>> = (0..READER.len())
        .map(|mode| reader_controls(ui, params, layer, mode))
        .collect();
    let shown = modes.remove(current);
    stack(vec![
        share(
            Share::Cells,
            stack(vec![
                switch_leaf(params, Switch::Reader(layer)),
                switch_leaf(params, Switch::Loop(layer)),
                switch_leaf(params, Switch::Direction(layer)),
            ]),
        ),
        tree::reserve(shown, modes),
    ])
}

/// What reader `mode` puts on the Playback card. Two rows, not four: Shape sits with size and rate —
/// the three are what a grain *is* — and the row below is what varies from one grain to the next.
fn reader_controls(
    ui: &Ui,
    params: &MxmCreativeSamplerParams,
    layer: usize,
    mode: usize,
) -> Node<Leaf> {
    let travels = mode >= 1;
    let grains = mode == 2;
    let mut shaping = vec!["transpose"];
    if travels {
        shaping.push("scanspeed");
    }
    if grains {
        shaping.extend(["grainsize", "grainrate", "grainshape"]);
    }
    let ids = |suffixes: &[&str]| -> Vec<&'static str> {
        suffixes
            .iter()
            .map(|suffix| layer_id(params, layer, suffix))
            .collect()
    };
    // Grain rate with its tempo sync beside it, and Grain shape after: one flat row, so a wide
    // card's spare width stays at its end.
    let mut body = vec![if grains {
        let shape = shaping.pop().expect("grain shape is last");
        row_gap(
            ui.spacing().item_spacing.x,
            vec![
                knob_row(ui, params, &ids(&shaping)),
                sync_beside(params, layer_id(params, layer, "grainsync")),
                knob_row(ui, params, &ids(&[shape])),
            ],
        )
    } else {
        knob_row(ui, params, &ids(&shaping))
    }];
    // **Scan speed's routes, under scan speed**, drawn under the same condition the knob is.
    if travels {
        body.push(routes_leaf(ui, params, scan_target(layer)));
    }
    if grains {
        body.push(knob_row(
            ui,
            params,
            &ids(&["pitchvar", "posvar", "timing", "width", "reversechance"]),
        ));
        // **The shape is a curve, so it is drawn as one**, from the DSP's own `grain_window`.
        body.push(display(
            Leaf::GrainEnvelope(layer),
            crate::visuals::envelope_min_width(),
            crate::visuals::ENVELOPE_HEIGHT,
        ));
        // **The overlap line never re-wraps** (design system §7.5): the card reserves its longest
        // reading — the ceiling's warning at the most grains asked — so dragging Grain size or rate
        // moves nothing.
        let (text, _) = overlap_reading(params, layer);
        let longest = format!(
            "{:.1} grains asked for, {:.0} played",
            9999.9,
            mxm_creative_sampler_dsp::MAX_OVERLAP
        );
        body.push(tree::reserve(
            overlap_leaf(layer, text),
            vec![overlap_leaf(layer, longest)],
        ));
    }
    stack(body)
}

/// A layer's overlap reading, as the tree measures it.
fn overlap_leaf(layer: usize, text: String) -> Node<Leaf> {
    leaf(
        Leaf::Overlap(layer),
        Kind::Text {
            text,
            font: Font::Body,
            flow: Flow::Wrap,
        },
    )
}

fn scan_target(layer: usize) -> usize {
    use mxm_creative_sampler_dsp::routing::target;
    if layer == 0 {
        target::SCAN_A
    } else {
        target::SCAN_B
    }
}

/// Everything a leaf draws with: the parameters and their host, the editor's selection and text
/// buffers, the asset snapshot and load state, and the acquisition callbacks. Measurement is not
/// interactive: nothing here runs to learn a size, so no callback needs a throwaway stand-in.
pub(crate) struct Live<'a, 'b> {
    pub params: &'a MxmCreativeSamplerParams,
    pub setter: &'a ParamSetter<'b>,
    pub text: &'a mut HashMap<&'static str, Option<String>>,
    /// The layer every card was built around this frame, and the one every leaf paints.
    pub layer: usize,
    /// A layer a chip's click chose. [`cards`] selects it once every card is drawn, so nothing drawn
    /// this frame can see it; the next frame's trees are built around it.
    pub chosen: Option<usize>,
    pub samples: [Option<&'a Sample>; 2],
    pub names: [&'a str; 2],
    pub status: &'a LoadState,
    pub waveform: &'a [[f32; 2]],
    pub playhead: Option<f32>,
    pub telemetry: Option<&'a crate::telemetry::Telemetry>,
    pub load: &'a mut dyn FnMut(usize, PathBuf),
    pub clear: &'a mut dyn FnMut(usize),
    pub generate: &'a mut dyn FnMut(usize, crate::generate::Recipe),
}

/// Draws one leaf, in the `Ui` the tree bounded to `rect`, through the bindings — so the controls,
/// their gestures and their names are exactly what they were.
pub(crate) fn paint(
    ui: &mut Ui,
    tokens: &Tokens,
    leaf: &Leaf,
    rect: Rect,
    live: &mut Live<'_, '_>,
) {
    let params = live.params;
    let setter = live.setter;
    // **One acquisition at a time.** A second `set_loading` while the first is in flight advances
    // the revision again, and an earlier staged snapshot could be published carrying the later
    // one's gestures — so the chips, the crosses, Load, Generate and Root's helpers wait it out.
    let interactive = !matches!(live.status, LoadState::Loading { .. });
    match *leaf {
        // Synced to a tempo, a rate reads its division; the host still reads its value.
        Leaf::Knob(id) => {
            let size = KNOB;
            let bound = bound(id, params);
            let division = {
                use nice_plug::prelude::Param as _;
                let synced: Option<(bool, &nice_plug::prelude::FloatParam, mxm_tempo::Ladder)> =
                    match id {
                        "lforate" => Some((
                            params.lfo1_sync.value(),
                            &params.lfo_rate,
                            crate::params::LFO_SYNC,
                        )),
                        "lfo2rate" => Some((
                            params.lfo2_sync.value(),
                            &params.lfo2_rate,
                            crate::params::LFO_SYNC,
                        )),
                        "agrainrate" => Some((
                            params.a_grain_sync.value(),
                            &params.a_grain_rate,
                            crate::params::GRAIN_SYNC,
                        )),
                        "bgrainrate" => Some((
                            params.b_grain_sync.value(),
                            &params.b_grain_rate,
                            crate::params::GRAIN_SYNC,
                        )),
                        _ => None,
                    };
                synced
                    .filter(|(on, _, _)| *on)
                    .and_then(|(_, param, ladder)| {
                        ladder.shown(
                            param.unmodulated_normalized_value(),
                            live.telemetry.and_then(|t| t.tempo.get()),
                            f64::from(param.preview_plain(0.0)),
                            f64::from(param.preview_plain(1.0)),
                        )
                    })
            };
            match division {
                Some(division) => bound.knob_with_reading(
                    ui,
                    tokens,
                    setter,
                    size,
                    rect.width(),
                    live.text,
                    division.label(),
                ),
                None => bound.knob(ui, tokens, setter, size, rect.width(), live.text),
            }
        }
        Leaf::Picture(id) => {
            crate::binding::sync_picture(ui, tokens, id, bound(id, params).param, setter);
        }
        Leaf::Fader(id) => bound(id, params).slider_vertical(
            ui,
            tokens,
            setter,
            live.text,
            fader_label(id).unwrap_or("?"),
            rect.width(),
            mxm_ui::control::FADER_HEIGHT,
        ),
        Leaf::Switch(switch) => {
            let param = switch.param(params);
            let label = crate::binding::layer_label(param.name());
            segmented_named(
                ui,
                tokens,
                switch.id(),
                param,
                Some(&label),
                switch.options(),
                switch.details(),
                setter,
                0.0,
            );
        }
        Leaf::Waves(id) => {
            let shape = bound(id, params);
            let label = crate::binding::layer_label(shape.param.name());
            segmented_waves_named(
                ui,
                tokens,
                shape.id,
                shape.param,
                Some(&label),
                &LFO_SHAPES,
                Some(KNOB),
                &LFO_DETAILS,
                setter,
            );
        }
        Leaf::Routes(target) => route_stack(ui, tokens, target, params, setter, live.text),
        Leaf::Envelope => envelope_shape(
            ui,
            tokens,
            params.attack.value(),
            params.decay.value(),
            params.sustain.value(),
            params.release.value(),
        ),
        Leaf::Chip(layer) => chip(ui, tokens, live, layer, interactive),
        Leaf::Remove(layer) => {
            // **A cross per chip, so each layer is emptied where it is named.**
            if mxm_ui::control::remove_mark(
                ui,
                tokens,
                MIN_TARGET,
                live.samples[layer].is_some(),
                // A layer is an asset rather than a parameter, so there is nothing here for a host
                // to automate and nothing for the dot to report.
                &format!("Empty layer {}", if layer == 0 { "A" } else { "B" }),
                false,
                "Empties this layer. The other one keeps playing.",
            ) && interactive
            {
                (live.clear)(layer);
            }
        }
        Leaf::Load => load_button(ui, live.layer, interactive),
        Leaf::Generate => generated_source(
            ui,
            tokens,
            interactive,
            live.layer,
            live.names,
            live.status,
            live.generate,
        ),
        Leaf::Waveform => waveform(ui, tokens, live, interactive),
        Leaf::Status => {
            let (text, _) = status_line(live.status);
            match live.status {
                LoadState::Idle => {
                    ui.label(egui::RichText::new(text).color(tokens.text_secondary));
                }
                LoadState::Failed { .. } => {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&text).color(tokens.danger))
                            .truncate(),
                    )
                    .on_hover_text(text);
                }
                _ => {
                    fitted(ui, &text);
                }
            }
        }
        Leaf::Octave(layer, up) => octave_button(ui, params, setter, layer, up, interactive),
        Leaf::Auto(layer) => {
            auto_button(ui, params, setter, live.samples[layer], layer, interactive);
        }
        Leaf::Heard(ref said) => caption(ui, tokens, said),
        Leaf::GrainEnvelope(layer) => crate::visuals::envelope(ui, tokens, shape_of(params, layer)),
        Leaf::Overlap(layer) => {
            let (text, over) = overlap_reading(params, layer);
            ui.label(egui::RichText::new(text).color(if over {
                tokens.danger
            } else {
                tokens.text_secondary
            }));
        }
    }
}

/// A layer's chip in the layer bar: a button of fixed [`CHIP_WIDTH`] naming what the layer holds,
/// which selects the layer and takes a drop. Drawn on the bar's line — a horizontal at the pointer
/// target, the idiom `browser.rs` uses — so its name sits centred in it as it always has.
fn chip(ui: &mut Ui, tokens: &Tokens, live: &mut Live<'_, '_>, layer: usize, interactive: bool) {
    ui.horizontal(|ui| {
        // **One height for everything on the line**, settled upward to the pointer target this
        // collection draws at.
        ui.spacing_mut().interact_size.y = MIN_TARGET;
        let name = live.names[layer];
        let selected = live.layer == layer;
        // **Always a button, and always the same size.** `selectable_label` drew the unselected one
        // as bare text, so B read as a caption rather than as the thing you click to get at B's
        // controls — and the pair resized as the names changed under them.
        let fill = if selected {
            tokens.accent.gamma_multiply(0.22)
        } else {
            tokens.surface_2
        };
        let response = ui.add(
            egui::Button::new(chip_label(name))
                .fill(fill)
                .stroke(Stroke::new(
                    1.0,
                    if selected {
                        tokens.accent
                    } else {
                        tokens.border
                    },
                ))
                .min_size(Vec2::new(CHIP_WIDTH, MIN_TARGET)),
        );
        let response = if name.is_empty() {
            response.on_hover_text("Drop a WAV here, or select this layer and press Load.")
        } else {
            response.on_hover_text(name)
        };
        if interactive && response.clicked() {
            live.chosen = Some(layer);
        }
        if interactive {
            accept_drop(ui, response.rect, layer, live.load);
        }
    });
}

/// Load, at the end of the layer bar. It follows the selection: it needs one target, and the bar
/// beside it names one. **The dialog never runs inside this frame**: `mxm_ui::offthread` runs it on
/// its own thread and the editor collects the pick on a later frame, and loads it exactly as it
/// loads a drop.
fn load_button(ui: &mut Ui, layer: usize, interactive: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = MIN_TARGET;
        let picking = mxm_ui::offthread::running::<(usize, PathBuf)>(ui.ctx(), load_picker());
        let load = ui
            .add_enabled(!picking, egui::Button::new(LOAD))
            .on_hover_text("Choose a WAV for the selected layer; dropping one on it is faster.");
        if interactive && load.clicked() {
            mxm_ui::offthread::start(ui.ctx(), load_picker(), move || {
                rfd::FileDialog::new()
                    .add_filter("Wave audio", &["wav", "wave"])
                    .pick_file()
                    .map(|path| (layer, path))
            });
        }
    });
}

/// The selected layer's waveform: **the region editor**, the drop target, and the grains the layer
/// is reading, drawn over it.
fn waveform(ui: &mut Ui, tokens: &Tokens, live: &mut Live<'_, '_>, interactive: bool) {
    let params = live.params;
    let layer = live.layer;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), WAVEFORM_HEIGHT),
        Sense::click_and_drag(),
    );
    let hovered_files = ui.input(|input| !input.raw.hovered_files.is_empty());
    let over = hovered_files && response.hovered();
    ui.painter().rect_filled(
        rect,
        5.0,
        if over {
            tokens.accent.gamma_multiply(0.18)
        } else {
            tokens.canvas
        },
    );
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(
            if over { 2.0 } else { 1.0 },
            if over { tokens.accent } else { tokens.border },
        ),
        egui::StrokeKind::Inside,
    );
    if live.waveform.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("DROP A WAV ON {}", if layer == 0 { "A" } else { "B" }),
            egui::FontId::proportional(16.0),
            tokens.text_secondary,
        );
    } else {
        let centre = rect.center().y;
        let half = rect.height() * 0.43;
        let step = rect.width() / live.waveform.len().max(1) as f32;
        // **One vertical segment per bin, from its lowest sample to its highest.** A single signed
        // value per bin drew one polyline through the middle of the waveform, which for anything
        // with a bipolar cycle — a saw above all — reads as a barcode rather than as a sound.
        let segments: Vec<egui::Shape> = live
            .waveform
            .iter()
            .map(|[low, high]| *low..=*high)
            .enumerate()
            .map(|(index, span)| {
                let x = rect.left() + (index as f32 + 0.5) * step;
                egui::Shape::line_segment(
                    [
                        Pos2::new(x, centre - span.end() * half),
                        Pos2::new(x, centre - span.start() * half),
                    ],
                    // **As wide as its bin, not a point wide**, so the bars abut instead of piling up
                    // and egui's antialiasing carries the sub-point ones.
                    Stroke::new(step.clamp(0.5, 2.0), tokens.accent),
                )
            })
            .collect();
        ui.painter().extend(segments);
        region_handles(
            ui,
            tokens,
            params,
            live.setter,
            rect,
            &response,
            layer,
            live.samples[layer],
            interactive,
        );
        // **The grains, over the waveform they are reading**: after the region handles so the
        // shading does not hide them, before the playhead so it stays on top of the cloud.
        crate::visuals::grains(ui, tokens, live.telemetry, rect);
        // **The playhead**, last, and only while a voice is actually reading.
        if let Some(position) = live.playhead.filter(|p| p.is_finite()) {
            let x = rect.left() + position.clamp(0.0, 1.0) * rect.width();
            ui.painter().line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, tokens.warning),
            );
        }
    }
    if interactive {
        accept_drop(ui, rect, layer, live.load);
    }
}

/// The app bar's master knob, outside the paging renderer, so it takes a key beyond the eight
/// page cards.
pub const MASTER: u64 = 65;

/// The master output, drawn in the app bar's right group.
///
/// A bar card of its own, because the keyboard cursor records a spot only where a card and a
/// parameter scope are both open, and the paging report that places every other card cannot see
/// the bar. See `mxm_ui::navigation::bar_card`.
pub fn master(
    ui: &mut Ui,
    tokens: &Tokens,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    text: &mut HashMap<&'static str, Option<String>>,
) {
    // **A knob does not fit an app bar, and neither does a stacked slider.** A knob was eighty
    // points against the bar's forty-four; the slider that replaced it was two lines as well — the
    // name over the track with the value above that — so Master stood taller than every other item
    // on the bar and none of them lined up. The owner's report was the whole top row reading as
    // different sizes, unaligned, and this control was most of it. `slider_inline` is the same
    // control laid out along the bar instead of across it.
    mxm_ui::navigation::bar_card(ui, MASTER, |ui| {
        ui.scope(|ui| {
            bound("master", params).slider_inline(ui, tokens, setter, text, 96.0);
        })
        .response
        .rect
    });
}

/// The amplitude envelope, drawn.
///
/// **Four isolated knobs are not an envelope.** The shape is the thing being edited, so the card
/// shows it; the knobs stay underneath because a shape cannot be typed into.
///
/// The time axis is **proportional with a fixed sustain segment**, so a 2 ms attack stays visible
/// beside an 8 s release — an absolute axis makes exactly the patch you want to see invisible.
///
/// Modelled on `mxm-mono-01`'s `visuals::envelope_shape`, which draws the same shape and adds a
/// live position marker from its envelope telemetry. This instrument publishes no envelope stage,
/// so it draws the shape alone — design system §9 asks a visualisation to degrade gracefully when
/// realtime data is not there. The two should become one shared widget when mono-01 is next opened.
fn envelope_shape(
    ui: &mut Ui,
    tokens: &Tokens,
    attack_s: f32,
    decay_s: f32,
    sustain: f32,
    release_s: f32,
) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 72.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, tokens.canvas);
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, tokens.border),
        egui::StrokeKind::Inside,
    );

    let total = (attack_s + decay_s + release_s).max(0.001);
    let held = 0.25_f32;
    let span = 1.0 - held;
    let a = attack_s / total * span;
    let d = decay_s / total * span;
    let r = release_s / total * span;
    let inset = 6.0;
    let x = |t: f32| rect.left() + inset + (rect.width() - inset * 2.0) * t.clamp(0.0, 1.0);
    let y = |v: f32| rect.bottom() - inset - (rect.height() - inset * 2.0) * v.clamp(0.0, 1.0);

    let attack_end = a;
    let decay_end = a + d;
    let sustain_end = decay_end + held;
    let release_end = (sustain_end + r).min(1.0);
    ui.painter().add(egui::Shape::line(
        vec![
            Pos2::new(x(0.0), y(0.0)),
            Pos2::new(x(attack_end), y(1.0)),
            Pos2::new(x(decay_end), y(sustain)),
            Pos2::new(x(sustain_end), y(sustain)),
            Pos2::new(x(release_end), y(0.0)),
        ],
        Stroke::new(2.0, tokens.mod_envelope),
    ));
    response.on_hover_text("The volume envelope's shape.");
}

/// The LFOs' six shapes, in `LfoShape`'s order: drawn, not named (design system §7.3), in two rows
/// of three beside the rate knob. They were a stepped knob that read its shape as a word.
/// What each LFO shape does, in [`LFO_SHAPES`]' order.
const LFO_DETAILS: [&str; 6] = [
    "A smooth wobble.",
    "Rises and falls in straight lines.",
    "Rises, then snaps back down.",
    "Falls, then snaps back up.",
    "Jumps between two values, like a trill.",
    "A new random value on every cycle.",
];

const LFO_SHAPES: [(Wave, &str); 6] = [
    (Wave::Sine, "Sine"),
    (Wave::Triangle, "Triangle"),
    (Wave::RampUp, "Ramp up"),
    (Wave::RampDown, "Ramp down"),
    (Wave::Square, "Square"),
    (Wave::Random, "Sample & hold"),
];

/// How many characters of a source name a layer chip shows before it elides.
///
/// Roughly what a [`CHIP_WIDTH`] chip holds. Chosen to that, not measured: a name whose twenty
/// characters are wider than the chip widens it, and the Sample card's computed floor with it, so
/// nothing is painted past the card.
const CHIP_NAME_BUDGET: usize = 20;

/// How wide a layer chip is drawn, whatever is in it.
///
/// Fixed, so the pair does not resize as names change under them and the Load button does not walk
/// across the card when a longer file is dropped.
const CHIP_WIDTH: f32 = 150.0;

/// A file name shortened from the middle, keeping both ends.
///
/// **Truncate, not extend.** `crates/ui/src/control.rs` records the rule and why it exists: a label
/// that asks for more width than its card has does not wrap, it pushes `min_rect` out, and the card
/// grows sideways over its neighbour. Cards are laid out by taffy from a fixed floor with
/// `flex_shrink: 0`, so the overflow paints straight through whatever card is next — which is
/// exactly what a 70-character sample-library name did to the Reader card.
///
/// **The middle goes, not the tail**, because a library name carries meaning at both ends: the
/// instrument at the front and the key, tempo or take at the back. `SIG_GV_75_vibraphone_…_Bbmin.wav`
/// is still identifiable; `SIG_GV_75_vibraphone_melody_r…` is not.
fn elided(name: &str, budget: usize) -> String {
    let count = name.chars().count();
    if count <= budget || budget < 3 {
        return name.to_owned();
    }
    let keep = budget - 1;
    // Weighted toward the front, but with enough tail to hold a key or tempo suffix and the
    // extension: `SIG_GV_75_vibra…_Bbmin.wav` rather than `SIG_GV_75_vibraph…bmin.wav`, which
    // clips the very token a library name ends with to be searchable by.
    let tail = keep * 2 / 5;
    let head = keep - tail;
    let head: String = name.chars().take(head).collect();
    let tail: String = name.chars().skip(count - tail).collect();
    format!("{head}…{tail}")
}

/// A one-line label that elides to the width it was given rather than growing past it.
///
/// This line carries a whole file name, so it is the other half of the same defect the chips had.
/// `Label::truncate` measures against the card's real allocation, which is the pixel backstop the
/// chips' character budget only approximates.
fn fitted(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Label::new(text).truncate())
        .on_hover_text(text)
}

/// Where the Load dialog's `(layer, path)` pick waits for the editor frame that collects it.
pub fn load_picker() -> egui::Id {
    egui::Id::new("mxm-creative-sampler-load")
}

/// Which region marker a drag on the waveform has hold of.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handle {
    Start,
    End,
    LoopStart,
    LoopEnd,
}

impl Handle {
    const ALL: [Self; 4] = [Self::Start, Self::End, Self::LoopStart, Self::LoopEnd];

    fn suffix(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
            Self::LoopStart => "loopstart",
            Self::LoopEnd => "loopend",
        }
    }

    /// Region bounds are filled; loop bounds are open. The brief asks for both on one canvas.
    fn filled(self) -> bool {
        matches!(self, Self::Start | Self::End)
    }
}

/// **The waveform is the region editor**, which is what the brief asked for and what four detached
/// percentage knobs were not.
///
/// Three things this must respect, all of them from `Region::new` in the DSP:
///
/// - **Start and End are order-free.** The DSP takes `min`/`max` of the pair, so dragging End past
///   Start swaps their roles rather than clamping. The handle follows its *parameter*, so it keeps
///   hold of the thing it grabbed and the labels cross over — which is honest about what the sound
///   will do.
/// - **Loop points are clamped into the region afterwards.** A loop handle dragged outside the
///   region is drawn where the DSP will actually read it, not where the pointer is, so the display
///   cannot promise something the sound does not do.
/// - Positions are `normalised × (len - 1)`, so a marker sits at `left + normalised × width`.
/// - **A drag takes the circle it is pressed on**, never a neighbour on the same line — see [`grab`].
///
/// The numeric knobs below stay the keyboard's path. **These markers deliberately do not open a
/// `navigation::at` scope**: they edit the same four parameters, and a second scope for the same id
/// would report as an unreachable duplicate in the coverage check.
#[allow(clippy::too_many_arguments)]
/// **Seventeen sources the instrument can make for itself**, beside the one it can be given.
///
/// A dropped file is the primary gesture and stays the primary gesture; this is what the instrument
/// starts on and what it can go back to. Each entry sweeps its own spectrum across its length, so
/// the readers turn the scan position into a timbre — which is the reason for a bank of morphing
/// sources rather than a rack of static shapes. See [`crate::generate`].
///
/// **One click, and the value is derived rather than remembered.** Design system §7.4 asks that the
/// selected value always be visible, and a pending choice that had not been applied would be a
/// control saying something untrue — worse still once a drop landed on the layer underneath it. So
/// the displayed entry is read back from the loaded source's *name*: it is true by construction, it
/// survives the editor closing and the patch reloading, and it costs no editor state and no
/// parameter. A layer holding a dropped file shows the non-recipe `Choose source` entry. That entry
/// is load-bearing: falling back to recipe zero made the dropped file impersonate Init saw, so
/// choosing Init saw again was not a selection change and did nothing until another recipe was
/// chosen first.
///
/// Picking replaces the selected layer, exactly as `Load` and a drop do — the layer's own cross
/// already empties it on one click with nothing but a tooltip, so a menu is a higher bar, not a
/// lower one. It is **disabled while a load is in flight** rather than ignored: the bank's staging
/// deliberately does not fail on a second stage before a commit, so an unguarded repeat would
/// simply hold the editor busy for as long as the picks took.
fn generated_source(
    ui: &mut Ui,
    tokens: &Tokens,
    interactive: bool,
    layer: usize,
    names: [&str; 2],
    status: &LoadState,
    generate: &mut dyn FnMut(usize, crate::generate::Recipe),
) {
    let labels = generated_labels();
    // **Matched without building every name.** `Recipe::name` allocates, and this runs on every
    // paint of the card; the name is the label plus the root it declares, so stripping that suffix
    // compares the two without a single allocation.
    let mut selected = generated_selection(names.get(layer).copied().unwrap_or_default());
    let busy = matches!(status, LoadState::Loading { .. });
    let sounding = Some(selected);
    let description = generated_recipe(selected).map_or(
        "Choose a source this instrument generates for itself.",
        |recipe| recipe.description(),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = mxm_ui::space::MIN_TARGET;
        ui.add_enabled_ui(interactive && !busy, |ui| {
            // The shared caret selector, not a local dropdown: design system §7.4 owns this
            // geometry for a bounded list and says an editor does not substitute its own.
            if mxm_ui::control::selector(
                ui,
                tokens,
                "Generate",
                &labels,
                &mut selected,
                sounding,
                Some(1),
                description,
            ) && interactive
                && !busy
                && let Some(recipe) = generated_recipe(selected)
            {
                generate(layer, recipe);
            }
        });
    });
}

/// The Generate selector's entries: the non-recipe prompt, then every recipe. Length inferred
/// rather than written: the bank has grown once already, and a count spelled out here is a second
/// place to forget. Slot zero is deliberately not a recipe: after a file drop, Init saw must remain
/// a selectable action rather than the false current value.
fn generated_labels() -> [&'static str; crate::generate::Recipe::ALL.len() + 1] {
    let mut labels = ["Choose source"; crate::generate::Recipe::ALL.len() + 1];
    for (label, recipe) in labels[1..].iter_mut().zip(crate::generate::Recipe::ALL) {
        *label = recipe.label();
    }
    labels
}

/// Picker index zero means that the selected layer does not currently hold a generated source.
fn generated_selection(name: &str) -> usize {
    let Some(shape) = name.strip_suffix(" C3") else {
        return 0;
    };
    crate::generate::Recipe::ALL
        .iter()
        .position(|recipe| recipe.label() == shape)
        .map_or(0, |index| index + 1)
}

/// Resolve a picker index while keeping its non-recipe prompt out of the generated bank.
fn generated_recipe(selection: usize) -> Option<crate::generate::Recipe> {
    selection
        .checked_sub(1)
        .and_then(|index| crate::generate::Recipe::ALL.get(index))
        .copied()
}

/// **Root, without having to name the note by ear.**
///
/// An untouched Root is a number that agrees with the sample only by luck, and the player is left
/// auditioning the keyboard against the source to find where it sits. This asks the question the
/// control is really asking — *what note is this?* — and answers it from the audio.
///
/// It reads from the **region start**, so pointing Start into the middle of a sample tunes to what
/// is there rather than to whatever came first.
///
/// **It says what it found, including when it found nothing.** A button that silently does nothing
/// is indistinguishable from one that is broken, and this one legitimately declines: silence, a
/// drum, a chord and noise have no single pitch to report. Refusing is the right answer there — a
/// wrong Root retunes the whole keyboard on a guess the player never made — but only if it is
/// visible.
///
/// **And the octave beside it, because the octave is the error that actually happens.** Every way
/// this instrument arrives at a Root guesses the octave last and worst: a file name carries `C` far
/// more often than it carries `C2`, and pitch detection's characteristic failure is a whole octave
/// rather than a wrong note — a rich harmonic series invites the octave below and a weak
/// fundamental the octave above. The note is usually right and the register is not, so the repair a
/// player reaches for is *the same note, an octave over*. Twelve semitones is one press here
/// instead of twelve of the knob's own steps or a typed number.
///
/// They write the same parameter the knob does, through the same begin/set/end gesture, so a host
/// records the change as an ordinary edit. Each disables itself at the end of the range rather than
/// clamping silently, because a button that still looks pressable and does nothing is the defect
/// this card already avoids elsewhere.
fn auto_button(
    ui: &mut Ui,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    sample: Option<&Sample>,
    layer: usize,
    interactive: bool,
) {
    let button = ui
        .add_enabled(interactive, egui::Button::new(AUTO))
        .on_hover_text(
            "Listens to the sample and sets Root to the note it finds; unpitched sounds are left alone.",
        );
    if interactive && button.clicked() {
        let root = bound(layer_id(params, layer, "root"), params);
        let start = bound(layer_id(params, layer, "start"), params)
            .param
            .normalised();
        let found = sample.and_then(|sample| mxm_creative_sampler_dsp::detect_root(sample, start));
        let said = match found {
            Some(note) => {
                root.param.begin(setter);
                // Auto answers a whole note, as it always has (plan §1.3 leaves a fractional Auto
                // open); Root's range is what normalises it.
                root.param.set(setter, note.round() / ROOT_RANGE);
                root.param.end(setter);
                format!("heard {}", note_name(note.round()))
            }
            None => "no clear pitch — Root left alone".to_owned(),
        };
        let revision = params.assets.revision();
        ui.ctx()
            .data_mut(|data| data.insert_temp(heard_id(layer), (revision, said)));
    }
}

/// One of the octave buttons beside Auto ([`auto_button`] says why they are there): Root a whole
/// octave down, or up when `up`, keeping its cents. Root is a linear 0..127 in semitones, so an
/// octave is twelve of those and the normalisation is the parameter's own range.
fn octave_button(
    ui: &mut Ui,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    layer: usize,
    up: bool,
    interactive: bool,
) {
    let root = bound(layer_id(params, layer, "root"), params);
    let (label, delta, hint) = if up {
        (OCT_UP, OCTAVE, "Moves Root up an octave.")
    } else {
        (OCT_DOWN, -OCTAVE, "Moves Root down an octave.")
    };
    let target = octave_target(root.param.normalised(), delta);
    let shift = ui
        .add_enabled(interactive && target.is_some(), egui::Button::new(label))
        .on_hover_text(hint);
    if let Some(target) = target
        && interactive
        && shift.clicked()
    {
        root.param.begin(setter);
        root.param.set(setter, target);
        root.param.end(setter);
    }
}

/// Where an octave button would put Root, normalised — or `None` when that octave is off the end.
///
/// **Refusing rather than clamping.** Clamping would move Root to 127 or 0, which is a different
/// note: a player who pressed *up an octave* and got a semitone would be worse off than one whose
/// press did nothing, and the button would give no sign either way. `None` disables it instead, so
/// the range is visible in the control.
///
/// **An octave keeps Root's cents** (D6). A Root between notes is an imported loop's pitch, and an
/// octave is the correction that gets used; rounding here would detune the loop the import tuned. A
/// Root on a note reads back a hair off it through the normalisation, and lands on the note.
fn octave_target(normalised: f32, delta: f32) -> Option<f32> {
    let note = normalised * ROOT_RANGE;
    let note = if (note - note.round()).abs() < ON_A_NOTE {
        note.round()
    } else {
        note
    };
    let target = note + delta;
    (0.0..=ROOT_RANGE)
        .contains(&target)
        .then_some(target / ROOT_RANGE)
}

/// How near a note, in semitones, a Root read back through its normalisation still counts as on it:
/// a tenth of a cent, far finer than any pitch an import writes and far coarser than `f32` noise.
const ON_A_NOTE: f32 = 1.0e-3;

/// Root's full span in semitones. Its parameter is a linear 0..127, so this is also what
/// normalises a plain note number — the same arithmetic `Auto` uses to write what it heard.
const ROOT_RANGE: f32 = 127.0;
/// One octave, in semitones.
const OCTAVE: f32 = 12.0;
/// **Root turns by whole notes, and holds cents between them** (D6): one note, normalised, is what a
/// drag or a key press moves it by; a coarse press moves an octave.
const ROOT_NOTE_STEP: f64 = 1.0 / ROOT_RANGE as f64;
const ROOT_OCTAVE_STEP: f64 = OCTAVE as f64 / ROOT_RANGE as f64;

/// A MIDI number as a note name, **C4 = 60** — the convention `Root` itself displays.
fn note_name(value: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let note = value.round().clamp(0.0, 127.0) as i32;
    format!("{}{}", NAMES[(note % 12) as usize], note / 12 - 1)
}

#[allow(clippy::too_many_arguments)]
fn region_handles(
    ui: &mut Ui,
    tokens: &Tokens,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    rect: Rect,
    response: &egui::Response,
    layer: usize,
    sample: Option<&Sample>,
    interactive: bool,
) {
    let prefix = if layer == 0 { "a" } else { "b" };
    let value = |handle: Handle| {
        bound(&format!("{prefix}{}", handle.suffix()), params)
            .param
            .normalised()
    };
    let x_of = |v: f32| rect.left() + v.clamp(0.0, 1.0) * rect.width();

    // The region the DSP will actually play, shaded so the excluded ends read as excluded.
    let (a, b) = (value(Handle::Start), value(Handle::End));
    let (lo, hi) = (a.min(b), a.max(b));
    for span in [
        Rect::from_x_y_ranges(rect.left()..=x_of(lo), rect.y_range()),
        Rect::from_x_y_ranges(x_of(hi)..=rect.right(), rect.y_range()),
    ] {
        if span.width() > 0.5 {
            ui.painter()
                .rect_filled(span, 0.0, tokens.canvas.gamma_multiply(0.55));
        }
    }

    // **The crossfade, drawn from the DSP's own arithmetic.** `loop_crossfade` is the function the
    // readers fit their seam with, so the shading is where the fade is heard: the stretch before Loop
    // end that fades out, and the loop's opening from Loop start that fades in over it — shorter when
    // the loop limits the time to half its length, and absent where the fade does nothing, a loop
    // that is not Forward or a Grain layer. **That absence is the indication**; the owner ruled prose
    // under the knobs out (see `character`), so the picture carries it. No line marks the return
    // point: an earlier one read as a start marker, and a note always starts at Start.
    if let Some(sample) = sample {
        let (reader, loop_mode, crossfade) = if layer == 0 {
            (
                params.a_reader.value().dsp(),
                params.a_loop.value().dsp(),
                params.a_loop_crossfade.value(),
            )
        } else {
            (
                params.b_reader.value().dsp(),
                params.b_loop.value().dsp(),
                params.b_loop_crossfade.value(),
            )
        };
        let probe = mxm_creative_sampler_dsp::LayerParams {
            reader,
            start: a,
            end: b,
            loop_start: value(Handle::LoopStart),
            loop_end: value(Handle::LoopEnd),
            loop_mode,
            loop_crossfade_s: crossfade,
            ..Default::default()
        };
        let fade = mxm_creative_sampler_dsp::loop_crossfade(sample, &probe);
        if fade.effective_s > 0.0 {
            let length = fade.return_point - fade.loop_start;
            for (from, to) in [
                (fade.loop_end - length, fade.loop_end),
                (fade.loop_start, fade.return_point),
            ] {
                let span = Rect::from_x_y_ranges(x_of(from)..=x_of(to), rect.y_range());
                if span.width() > 0.5 {
                    ui.painter()
                        .rect_filled(span, 0.0, tokens.accent.gamma_multiply(0.16));
                }
            }
        }
    }

    // **Where a circle is drawn is where it is grabbed**, from one set of positions for both: region
    // handles filled at the top, loop handles hollow at the bottom, a loop handle inside the region
    // where the DSP reads it.
    let circles = Handle::ALL.map(|handle| {
        let raw = value(handle);
        let shown = match handle {
            Handle::Start | Handle::End => raw,
            _ => raw.clamp(lo, hi),
        };
        let y = if handle.filled() {
            rect.top() + 4.0
        } else {
            rect.bottom() - 4.0
        };
        Pos2::new(x_of(shown), y)
    });

    let mut held = ui.ctx().data(|d| d.get_temp::<usize>(handle_id(rect)));
    if interactive {
        if response.drag_started()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            // The press says which circle was taken, and the distance travelled since — the drag
            // threshold's worth — says which way it is going.
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
            let index = grab(circles, press, rect, pointer.x - press.x);
            bound(&format!("{prefix}{}", Handle::ALL[index].suffix()), params)
                .param
                .begin(setter);
            ui.ctx().data_mut(|d| d.insert_temp(handle_id(rect), index));
            // **Held from this frame, not the next.** Memory is read before the grab, so the handle
            // taken now was only known next frame: this frame's movement wrote nothing, and a drag
            // that began and ended in one frame never closed the gesture it opened — a code review's
            // finding, and a latched automation lane in a host.
            held = Some(index);
        }
        if let (true, Some(index), Some(pointer)) =
            (response.dragged(), held, response.interact_pointer_pos())
        {
            let t = ((pointer.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
            bound(&format!("{prefix}{}", Handle::ALL[index].suffix()), params)
                .param
                .set(setter, t);
        }
        if response.drag_stopped() {
            if let Some(index) = held {
                bound(&format!("{prefix}{}", Handle::ALL[index].suffix()), params)
                    .param
                    .end(setter);
            }
            ui.ctx().data_mut(|d| d.remove::<usize>(handle_id(rect)));
        }
    }

    for (handle, circle) in Handle::ALL.into_iter().zip(circles) {
        ui.painter().line_segment(
            [
                Pos2::new(circle.x, rect.top()),
                Pos2::new(circle.x, rect.bottom()),
            ],
            Stroke::new(1.0, tokens.text_secondary.gamma_multiply(0.8)),
        );
        if handle.filled() {
            ui.painter().circle_filled(circle, 4.0, tokens.accent);
        } else {
            ui.painter()
                .circle_stroke(circle, 4.0, Stroke::new(1.5, tokens.accent));
        }
    }
}

/// Which handle a drag takes hold of, as an index into [`Handle::ALL`].
///
/// **The circle the press lands on, in its own row.** The owner's report (2026-09-14): with Start and
/// Loop start on one line, taking the hollow circle moved Start, and the two moved together until they
/// no longer touched. The grab measured only the distance across the waveform, so two handles on one
/// line tied and the first in the list won — and the loop handle then followed Start, because it is
/// drawn inside the region. Now the press's half of the canvas picks the row, filled region handles at
/// the top and hollow loop handles at the bottom, and its position picks the circle in that row,
/// measured where the circles are drawn.
///
/// **Two circles on one spot go the way the drag goes**: right takes End or Loop end, left takes
/// Start or Loop start — the handle a player pulling that way is reaching for.
fn grab(circles: [Pos2; 4], press: Pos2, rect: Rect, heading: f32) -> usize {
    let row = if press.y < rect.center().y {
        [Handle::Start, Handle::End]
    } else {
        [Handle::LoopStart, Handle::LoopEnd]
    };
    let [opening, closing] =
        row.map(|handle| Handle::ALL.iter().position(|h| *h == handle).unwrap_or(0));
    let away = |index: usize| (circles[index].x - press.x).abs();
    if (away(opening) - away(closing)).abs() < 0.5 {
        if heading > 0.0 { closing } else { opening }
    } else if away(opening) < away(closing) {
        opening
    } else {
        closing
    }
}

fn handle_id(rect: Rect) -> egui::Id {
    egui::Id::new(("mxm-region-handle", rect.left() as i32, rect.top() as i32))
}

fn accept_drop(ui: &Ui, rect: Rect, layer: usize, load: &mut dyn FnMut(usize, PathBuf)) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    if !pointer.is_some_and(|position| rect.contains(position)) {
        return;
    }
    let dropped = ui.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .collect::<Vec<_>>()
    });
    if let Some(path) = dropped.into_iter().next() {
        load(layer, path);
    }
}

/// A quiet secondary line. Same shape as `mxm-mono-08`'s, which the design system's "Also from"
/// rule established.
fn caption(ui: &mut Ui, tokens: &Tokens, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(tokens.text_secondary)
            .text_style(mxm_ui::typography::caption_style(ui.style())),
    );
}

/// The grain shape the layer on show is sitting at.
fn shape_of(params: &MxmCreativeSamplerParams, layer: usize) -> f32 {
    if layer == 0 {
        params.a_grain_shape.value()
    } else {
        params.b_grain_shape.value()
    }
}

/// How many grains the current size and density ask to have sounding, against the ceiling.
///
/// **The ceiling used to be silent**, and a control with inert travel that nobody is told about is
/// an interface defect rather than a tuning question.
///
/// It also used to be *re-derived here*, from raw parameter values without the DSP's sanitise and
/// clamp, so this readout and the gain law could disagree — below one grain of overlap and above
/// the ceiling, they did. The crate DOX claimed they could not. It calls
/// [`mxm_creative_sampler_dsp::overlap`] now, which is the one place that arithmetic lives. The
/// reading is inked as a danger when more is asked for than is played.
fn overlap_reading(params: &MxmCreativeSamplerParams, layer: usize) -> (String, bool) {
    let (size, density) = if layer == 0 {
        (params.a_grain_size.value(), params.a_grain_rate.value())
    } else {
        (params.b_grain_size.value(), params.b_grain_rate.value())
    };
    let probe = mxm_creative_sampler_dsp::LayerParams {
        grain_size_s: size,
        grain_density_hz: density,
        ..Default::default()
    };
    let asked = mxm_creative_sampler_dsp::overlap(&probe, density);
    let raw = size * density;
    let ceiling = mxm_creative_sampler_dsp::MAX_OVERLAP;
    let over = raw > ceiling;
    let text = if over {
        format!("{raw:.1} grains asked for, {ceiling:.0} played")
    } else {
        format!("{asked:.1} of {ceiling:.0} grains overlapping")
    };
    (text, over)
}

/// One target's route rows and the affordance that adds another.
///
/// **Routing belongs under the thing it affects**, never in a detached footer — the ruling
/// `plugins/mxm-mono-00/AGENTS.md` records and design system §7.4 makes normative. The rows, the
/// border rules and the `‹ modulate ›` menu all come from `mxm_modulation_params`, so every editor
/// in the collection draws this the same way: an unrouted target draws **no group at all**, the menu
/// sits **outside** the group, and a row ends in a remove cross rather than a switch.
fn route_stack(
    ui: &mut Ui,
    tokens: &Tokens,
    target: usize,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    text: &mut HashMap<&'static str, Option<String>>,
) {
    let target_routes = params.routes.target(target);
    let entry = text.entry(ROUTE_TEXT_KEYS[target]).or_default();
    mxm_modulation_params::ui::stack(
        ui,
        tokens,
        mxm_creative_sampler_dsp::routing::TARGET_NAMES[target],
        // Nothing to drop: these names carry no module prefix the card already says.
        panel_name(target),
        &target_routes,
        entry,
        setter,
    );
}

/// One text-entry key per target, because a shared key would let a half-typed depth on one stack
/// appear in another.
const ROUTE_TEXT_KEYS: [&str; mxm_creative_sampler_dsp::routing::TARGETS] = [
    "routes_amp",
    "routes_cutoff",
    "routes_pitch",
    "routes_scana",
    "routes_scanb",
];

#[cfg(test)]
mod generated_source_tests {
    use super::{generated_recipe, generated_selection};
    use crate::generate::Recipe;

    /// **A dropped file is not Init saw.** Recipe zero used to be both the fallback picker index and
    /// Init saw's index, so selecting Init saw after a drop reported no change and never generated
    /// anything. The prompt owns index zero now and every real recipe is offset by one.
    #[test]
    fn a_drop_leaves_init_saw_selectable_in_one_step() {
        assert_eq!(generated_selection("field recording.wav"), 0);
        assert_eq!(generated_recipe(0), None);
        assert_eq!(generated_recipe(1), Some(Recipe::InitSaw));
        assert_eq!(generated_selection("Init saw C3"), 1);
        assert_eq!(
            generated_recipe(generated_selection("Supersaw C3")),
            Some(Recipe::Supersaw)
        );
    }
}

#[cfg(test)]
mod elision_tests {
    use super::{CHIP_NAME_BUDGET, elided};

    /// The name from the report that broke the card: 70 characters of sample-library convention.
    const LONG: &str = "SIG_GV_75_vibraphone_melody_roll_ambient_jazz_ensemble_isitme_Bbmin.wav";

    #[test]
    fn a_long_name_is_bounded_and_keeps_both_ends() {
        let short = elided(LONG, CHIP_NAME_BUDGET);
        assert_eq!(
            short.chars().count(),
            CHIP_NAME_BUDGET,
            "the chip name is still {} characters: {short}",
            short.chars().count()
        );
        assert!(
            short.starts_with("SIG_GV_75"),
            "the instrument at the front was lost: {short}"
        );
        assert!(
            short.ends_with(".wav"),
            "the extension at the back was lost: {short}"
        );
        assert!(short.contains('…'), "nothing marks the elision: {short}");
    }

    #[test]
    fn a_name_that_fits_is_left_alone() {
        for name in ["kick.wav", "a", ""] {
            assert_eq!(elided(name, CHIP_NAME_BUDGET), name);
        }
    }

    /// Elision counts characters, not bytes, so a multi-byte name must not panic or split a glyph.
    #[test]
    fn a_multibyte_name_survives_elision() {
        let name = "vibrafón_mélodie_ambiante_jazz_ensemble_ré♭min.wav";
        let short = elided(name, CHIP_NAME_BUDGET);
        assert_eq!(short.chars().count(), CHIP_NAME_BUDGET);
        assert!(short.ends_with(".wav"), "{short}");
    }
}

#[cfg(test)]
mod floor_tests {
    use super::*;
    use crate::asset::LoadState;

    use mxm_plugin_test::keyboard_checks;

    /// Every text shape card `index` paints at `width`, drawn through its tree as the editor draws
    /// it, with where each landed.
    fn painted_text(
        width: f32,
        index: usize,
        params: &MxmCreativeSamplerParams,
        shown: &Shown<'_>,
        samples: [Option<&Sample>; 2],
    ) -> Vec<(String, Rect)> {
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let ctx = egui::Context::default();
        mxm_ui::theme::apply(&ctx);
        mxm_ui::typography::apply(&ctx);
        let mut boxes = Vec::new();
        for pass in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(width + 400.0, 2400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let tokens = mxm_ui::LIGHT;
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 2400.0))),
                    );
                    let mut text = HashMap::new();
                    let mut load = |_: usize, _: PathBuf| {};
                    let mut clear = |_: usize| {};
                    let mut generate = |_: usize, _: crate::generate::Recipe| {};
                    let mut live = Live {
                        params,
                        setter: &setter,
                        text: &mut text,
                        layer: shown.layer,
                        chosen: None,
                        samples,
                        names: shown.names,
                        status: shown.status,
                        waveform: &[[0.0, 0.0]; 64],
                        playhead: None,
                        telemetry: None,
                        load: &mut load,
                        clear: &mut clear,
                        generate: &mut generate,
                    };
                    mxm_ui::ModuleCard::new(TITLES[index]).show(&mut child, &tokens, |ui| {
                        let body = card(ui, index, params, shown);
                        tree::show(ui, &tokens, &body, |ui, leaf, rect| {
                            paint(ui, &tokens, leaf, rect, &mut live);
                        });
                    });
                },
            );
            if pass == 2 {
                for clipped in &output.shapes {
                    collect_text(&clipped.shape, &mut boxes);
                }
            }
            output.textures_delta.clear();
        }
        boxes
    }

    fn collect_text(shape: &egui::epaint::Shape, boxes: &mut Vec<(String, Rect)>) {
        match shape {
            egui::epaint::Shape::Text(text) => boxes.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, boxes);
                }
            }
            _ => {}
        }
    }

    /// **The Sample card reads region, then layer, then Root** — the owner's layout (2026-09-13):
    /// Root after Pan, and the loop crossfade after Loop end. Asserted on the labels as painted, row by
    /// row, because a list in the source says nothing about where a knob lands.
    #[test]
    fn the_sample_card_puts_root_after_pan_and_the_crossfade_after_loop_end() {
        let params = MxmCreativeSamplerParams::default();
        let sample = Sample::new(vec![[0.0, 0.0]; 256], 48_000.0).unwrap();
        let shown = Shown {
            layer: 0,
            loaded: [true, true],
            names: ["a.wav", "b.wav"],
            status: &LoadState::Idle,
        };
        let floor = test_items(&params, &shown)[3].card.floor;
        let boxes = painted_text(floor, 3, &params, &shown, [Some(&sample), Some(&sample)]);
        let at = |label: &str| {
            boxes
                .iter()
                .find(|(text, _)| text == label)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| {
                    let painted: Vec<_> = boxes.iter().map(|(text, _)| text.as_str()).collect();
                    panic!("`{label}` was not painted; the card painted {painted:?}")
                })
        };
        let same_row = |one: Rect, other: Rect| (one.center().y - other.center().y).abs() < 2.0;

        // `Crossfade`, not its host name: see `binding::layer_label` for why it had to lose the word.
        let region = ["Start", "End", "Loop start", "Loop end", "Crossfade"].map(at);
        for pair in region.windows(2) {
            assert!(
                same_row(pair[0], pair[1]),
                "the region row is not one row: {region:?}"
            );
            assert!(
                pair[0].center().x < pair[1].center().x,
                "the region row is out of order"
            );
        }
        let layer = ["Level", "Pan", "Root"].map(at);
        for pair in layer.windows(2) {
            assert!(same_row(pair[0], pair[1]), "the layer row is not one row");
            assert!(
                pair[0].center().x < pair[1].center().x,
                "Root does not follow Pan"
            );
        }
        assert!(
            layer[0].top() > region[0].bottom(),
            "the layer row is not below the region row"
        );
        assert!(
            at("Auto").top() > layer[2].bottom(),
            "Auto did not move under the row that holds Root"
        );
    }

    /// **A drag takes the circle it was pressed on** — the owner's report (2026-09-14): with Start and
    /// Loop start on one line, taking the hollow circle moved Start, and both moved until they parted.
    #[test]
    fn a_drag_takes_the_circle_it_was_pressed_on() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 154.0));
        let (top, bottom) = (rect.top() + 4.0, rect.bottom() - 4.0);
        let index = |handle: Handle| Handle::ALL.iter().position(|h| *h == handle).unwrap();

        // Start and Loop start on the left edge, End and Loop end on the right: the import default.
        let stacked = [
            Pos2::new(0.0, top),
            Pos2::new(400.0, top),
            Pos2::new(0.0, bottom),
            Pos2::new(400.0, bottom),
        ];
        assert_eq!(
            grab(stacked, Pos2::new(2.0, top), rect, 5.0),
            index(Handle::Start)
        );
        assert_eq!(
            grab(stacked, Pos2::new(2.0, bottom), rect, 5.0),
            index(Handle::LoopStart),
            "the hollow circle on Start's line took Start"
        );
        assert_eq!(
            grab(stacked, Pos2::new(398.0, top), rect, -5.0),
            index(Handle::End)
        );
        assert_eq!(
            grab(stacked, Pos2::new(398.0, bottom), rect, -5.0),
            index(Handle::LoopEnd),
            "the hollow circle on End's line took End"
        );

        // Loop start and Loop end on one spot: the way the drag goes decides.
        let pinched = [
            Pos2::new(0.0, top),
            Pos2::new(400.0, top),
            Pos2::new(200.0, bottom),
            Pos2::new(200.0, bottom),
        ];
        assert_eq!(
            grab(pinched, Pos2::new(201.0, bottom), rect, 6.0),
            index(Handle::LoopEnd)
        );
        assert_eq!(
            grab(pinched, Pos2::new(199.0, bottom), rect, -6.0),
            index(Handle::LoopStart)
        );
        // A purely vertical drag has no heading, and takes the opening handle.
        assert_eq!(
            grab(pinched, Pos2::new(200.0, bottom), rect, 0.0),
            index(Handle::LoopStart)
        );

        // Apart, the nearest circle in the pressed row wins, whatever the other row holds.
        let apart = [
            Pos2::new(0.0, top),
            Pos2::new(400.0, top),
            Pos2::new(100.0, bottom),
            Pos2::new(300.0, bottom),
        ];
        assert_eq!(
            grab(apart, Pos2::new(120.0, bottom), rect, 0.0),
            index(Handle::LoopStart)
        );
        assert_eq!(
            grab(apart, Pos2::new(290.0, top), rect, 0.0),
            index(Handle::End),
            "a press in the top row took a loop handle"
        );
    }

    /// **A waveform drag is one balanced gesture that writes from its first frame** — a code review's
    /// finding (2026-09-14): the handle taken on a drag's first frame was remembered only for the
    /// next, so that frame wrote nothing, and the shortest drag — stopped on the frame after it
    /// started — wrote nothing at all.
    ///
    /// **egui 0.36 never starts a drag in the frame the button is released** (`interaction.rs`: a
    /// release clears the drag before one can begin). So the shortest drag is press, one move,
    /// release; and a press and release in one frame opens no gesture at all, which is asserted
    /// rather than skipped — a round of review found the earlier `if begins > 0` passing vacuously.
    #[test]
    fn a_waveform_drag_writes_from_its_first_frame_and_always_closes() {
        let params = MxmCreativeSamplerParams::default();
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), egui::vec2(400.0, 154.0));
        let press = Pos2::new(24.0, rect.bottom() - 6.0);
        let primary = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let drag = |frames: Vec<Vec<egui::Event>>| {
            let host = keyboard_checks::Recorder::default();
            let setter = ParamSetter::new(&host);
            let ctx = egui::Context::default();
            for events in std::iter::once(Vec::new()).chain(frames) {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(800.0, 400.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let tokens = mxm_ui::LIGHT;
                        let response = ui.interact(
                            rect,
                            egui::Id::new("waveform-under-test"),
                            Sense::click_and_drag(),
                        );
                        region_handles(
                            ui, &tokens, &params, &setter, rect, &response, 0, None, true,
                        );
                    },
                );
                output.textures_delta.clear();
            }
            (host.begins(), host.sets(), host.ends())
        };

        // Across frames: press, move, move, release.
        let (begins, sets, ends) = drag(vec![
            vec![egui::Event::PointerMoved(press), primary(press, true)],
            vec![egui::Event::PointerMoved(press + egui::vec2(40.0, 0.0))],
            vec![egui::Event::PointerMoved(press + egui::vec2(80.0, 0.0))],
            vec![primary(press + egui::vec2(80.0, 0.0), false)],
        ]);
        assert_eq!(
            (begins, ends),
            (1, 1),
            "the drag was not one balanced gesture"
        );
        assert!(
            sets >= 2,
            "a drag that moved on two frames wrote {sets} time(s): its first frame wrote nothing"
        );

        // The shortest drag egui has: press, one move, release. Its one moving frame is the frame the
        // drag starts on, so whatever it writes, it writes there.
        let (begins, sets, ends) = drag(vec![
            vec![egui::Event::PointerMoved(press), primary(press, true)],
            vec![egui::Event::PointerMoved(press + egui::vec2(60.0, 0.0))],
            vec![primary(press + egui::vec2(60.0, 0.0), false)],
        ]);
        assert_eq!(
            (begins, ends),
            (1, 1),
            "the shortest drag was not one balanced gesture"
        );
        assert!(
            sets >= 1,
            "the shortest drag wrote nothing: its only moving frame was its first"
        );

        // Moved and released within one frame: not a drag, so nothing opens and nothing is left open.
        let gesture = drag(vec![
            vec![egui::Event::PointerMoved(press), primary(press, true)],
            vec![
                egui::Event::PointerMoved(press + egui::vec2(60.0, 0.0)),
                primary(press + egui::vec2(60.0, 0.0), false),
            ],
        ]);
        assert_eq!(
            gesture,
            (0, 0, 0),
            "a press and release in one frame opened a gesture"
        );
    }

    /// An octave is twelve semitones and the ends of the range refuse rather than clamp, because a
    /// clamped press would land on a different note than the one asked for and say nothing about it.
    #[test]
    fn an_octave_button_moves_twelve_semitones_and_stops_at_the_ends() {
        let at = |note: f32| note / ROOT_RANGE;
        // C3 is 48 in this instrument's convention; up an octave is C4.
        assert_eq!(octave_target(at(48.0), OCTAVE), Some(at(60.0)));
        assert_eq!(octave_target(at(48.0), -OCTAVE), Some(at(36.0)));
        // Exactly reachable at both ends.
        assert_eq!(octave_target(at(12.0), -OCTAVE), Some(at(0.0)));
        assert_eq!(octave_target(at(115.0), OCTAVE), Some(at(127.0)));
        // And refused past them, rather than clamped onto a note nobody asked for.
        assert_eq!(octave_target(at(11.0), -OCTAVE), None);
        assert_eq!(octave_target(at(116.0), OCTAVE), None);
        assert_eq!(octave_target(at(0.0), -OCTAVE), None);
        assert_eq!(octave_target(at(127.0), OCTAVE), None);
        // A Root between notes — an imported loop's pitch — keeps its cents an octave away (D6).
        let kept = octave_target(at(59.75), OCTAVE).expect("an octave up from 59.75 is in range");
        assert!(
            (kept * ROOT_RANGE - 71.75).abs() < 1.0e-3,
            "an octave lost Root's cents: {}",
            kept * ROOT_RANGE
        );
        assert_eq!(octave_target(at(115.5), OCTAVE), None);
    }

    /// **The widest card must fit the narrowest window**, or it is drawn past the right edge and
    /// cannot be reached: the flow ships `Scroll::Vertical`, which has no horizontal scroll to get
    /// to it. `crates/ui/src/flow.rs` states that trade-off on `Scroll`.
    #[test]
    fn the_widest_card_fits_the_minimum_window() {
        let params = MxmCreativeSamplerParams::default();
        let items = test_items(
            &params,
            &Shown {
                layer: 0,
                loaded: [true, true],
                names: ["a.wav", "b.wav"],
                status: &LoadState::Idle,
            },
        );
        let cards: Vec<_> = items.iter().map(|item| item.card).collect();
        let widest = mxm_ui::flow::minimum_width(&cards);
        let minimum = crate::editor::MINIMUM.0 as f32;
        println!(
            "widest card floor {widest}, minimum window {minimum}, slack {}",
            minimum - widest
        );
        assert!(
            widest <= minimum,
            "the widest card floor {widest} exceeds the minimum window {minimum}"
        );
    }
}
