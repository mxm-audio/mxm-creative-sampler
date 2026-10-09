//! Binding one parameter to one `mxm-ui` control: **the collection's one binding**,
//! `mxm_preset::binding` (`plans/plan-editor-standard.md` R1c). Every editor carried its own copy
//! of it until 2026-09-24; this module re-exports the one, so `super::binding::…` paths keep
//! meaning what they did.

pub use mxm_preset::binding::*;

/// A parameter name with the context its card already supplies removed.
///
/// Two things get stripped, for one reason. The `A `/`B ` layer prefix goes because the layer bar
/// says which layer is selected. `Converter ` goes because the Converter card is titled — the knob
/// under that heading does not need to repeat it, and the owner asked for the short form.
///
/// **The host name is untouched either way.** `Converter drive` and the Filter card's `Drive` are
/// two different automatable parameters and a DAW's list has to tell them apart; it is only the
/// panel, where the heading is right there, that can afford the short one. Design system §7.1 draws
/// exactly this line between `name` and `label`.
///
/// **And the module its card is titled by** (design system §7.1): *Rate 1* and *Shape 1* on a card
/// titled *LFOs*, where the number is what tells the two LFOs apart; *Mode* on *Playback* and on
/// *Filter*.
pub(crate) fn layer_label(name: &str) -> String {
    match name {
        "LFO 1 rate" => return "Rate 1".to_owned(),
        "LFO 1 shape" => return "Shape 1".to_owned(),
        "LFO 2 rate" => return "Rate 2".to_owned(),
        "LFO 2 shape" => return "Shape 2".to_owned(),
        "Filter mode" => return "Mode".to_owned(),
        name if name.ends_with("Playback mode") => return "Mode".to_owned(),
        _ => {}
    }
    if let Some(rest) = name.strip_prefix("Converter ") {
        // Sentence case survives the strip: "Converter drive" is one sentence-cased name, so the
        // word left behind has to be re-capitalised or the panel reads "drive".
        let mut chars = rest.chars();
        return match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => name.to_owned(),
        };
    }
    let local = name
        .strip_prefix("A ")
        .or_else(|| name.strip_prefix("B "))
        .unwrap_or(name);
    // **`Loop crossfade` draws as `Crossfade`.** At full length it is the one label on the Sample
    // card that wraps to two lines in a knob column, which made the region row taller than the row
    // it belongs beside — `the_sample_card_puts_root_after_pan_and_the_crossfade_after_loop_end`
    // measured 32 points against 16. It sits after Loop start and Loop end, so the loop is already
    // named on the row; the host name keeps the word, because an automation list has no row.
    if local == "Loop crossfade" {
        return "Crossfade".to_owned();
    }
    local.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::MxmCreativeSamplerParams;

    /// Root's whole-note override still decides where an arrow lands, whatever law the binding
    /// carries: a note fine and an octave coarse, not the parameter's own tiny continuous step.
    #[test]
    fn roots_note_override_outranks_the_keyboard_law() {
        use mxm_ui::control::{NextValue, Press};

        let params = MxmCreativeSamplerParams::default();
        let note = 1.0 / 127.0;
        let bound = Bound::new("aroot", &params.a_root, "")
            .stepped(note, 12.0 * note)
            .law(StepLaw::Semitones);
        let from = 60.0 * note;
        let fine = bound.next_value(
            from,
            Press {
                up: true,
                coarse: false,
                finer: false,
                snap: false,
            },
        );
        let coarse = bound.next_value(
            from,
            Press {
                up: false,
                coarse: true,
                finer: false,
                snap: false,
            },
        );
        assert!((fine - 61.0 * note).abs() < 1e-9, "a note up: {fine}");
        assert!(
            (coarse - 48.0 * note).abs() < 1e-9,
            "an octave down: {coarse}"
        );
    }
}
