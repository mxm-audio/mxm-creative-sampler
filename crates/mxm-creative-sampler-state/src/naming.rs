//! What a sample's file name says about its pitch.
//!
//! **There is a standard for this and it is empty.** RIFF's `smpl` chunk carries
//! `dwMIDIUnityNote` — the MIDI note at which the file plays unpitched, middle C being 60 — and it
//! is exactly the field this instrument wants. Measured across 600 files drawn at random from the
//! owner's library: **17 carried a `smpl` chunk at all, and all seventeen had `dwMIDIUnityNote`
//! set to 0.** Zero is C-1, five octaves below anything in a bass pack; it is the field being
//! present and unset rather than the field meaning what it says. Reading it would be correct by the
//! specification and would tune a sample to the bottom of the keyboard, so this does not read it.
//! `crate::wav_loop` reads it only where it is set, and never as Root: it counts the cycles in a loop
//! the file carries, and the loop's own length gives the note.
//!
//! **The de-facto standard is the file name**, and it is consistent enough to parse. Across 6,941
//! WAV files in that library, about 15% end in a key tag, in these shapes:
//!
//! | shape | seen | example |
//! |---|---|---|
//! | bare letter | 338 | `..._C`, `..._A` |
//! | note + mode | 338 | `..._Am`, `..._Cm`, `..._Fmin` |
//! | note + accidental | 142 | `..._Eb`, `..._Bb`, `..._F#` |
//! | note + accidental + mode | 186 | `..._G#m`, `..._C#min`, `..._Ebmaj` |
//! | note + octave | 26 | `..._C3` |
//!
//! **Two facts from that survey decide the whole design.** The octave is almost never there — 26 of
//! roughly a thousand — so a name gives a *pitch class* and not a note. And every one of the 338
//! bare single letters is **uppercase and in A–G**, with no lowercase last token anywhere in the
//! library: a bare `_A` is a key and never a take marker, which is the false positive that would
//! otherwise make this idea unusable.

/// What a file name claims about its pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyTag {
    /// 0 is C, 11 is B.
    pub class: u8,
    /// Present in about one name in forty, and authoritative when it is.
    pub octave: Option<i32>,
}

/// The key a file name claims, or `None` when it claims nothing.
///
/// The key is conventionally the **last** token, so only the last is read: a name like
/// `jazz_Bm_piano_smoke` would be a guess, and a guess that retunes the keyboard is worse than
/// leaving Root where it is.
#[must_use]
pub fn key_in_name(stem: &str) -> Option<KeyTag> {
    let token = stem.rsplit(['_', '-', ' ']).next()?.trim();
    let mut chars = token.chars();

    // **Uppercase, always.** The survey found 338 bare single letters and not one lowercase last
    // token; lowering the bar here is what would turn a `take_b` into a B.
    let letter = chars.next()?;
    let class = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };

    let rest: String = chars.collect();
    let (accidental, rest) = match rest.strip_prefix('#') {
        Some(rest) => (1i32, rest),
        None => match rest.strip_prefix('b') {
            Some(rest) => (-1, rest),
            None => (0, rest.as_str()),
        },
    };

    // An octave, when there is one. A leading `-` is allowed because C-1 is a real note name.
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    let rest = &rest[digits.len()..];
    let octave = if digits.is_empty() {
        None
    } else {
        Some(digits.parse::<i32>().ok()?)
    };

    // Whatever is left has to be a mode, or this token is not a key. `m`, `min`, `maj` — a mode
    // says nothing about pitch and is dropped, but an unrecognised tail means the token was never
    // a key in the first place and must not be read as one.
    match rest {
        "" | "m" | "M" | "min" | "maj" | "MIN" | "MAJ" | "Min" | "Maj" => {}
        _ => return None,
    }

    Some(KeyTag {
        class: (class + accidental).rem_euclid(12) as u8,
        octave,
    })
}

/// The MIDI note a [`KeyTag`] means, given what the audio itself sounds like.
///
/// **The name supplies the pitch class and the audio supplies the octave.** A name almost never
/// carries an octave, and a pitch class on its own is not a root — a bass one-shot called `_F` is
/// F1, and the same name on a flute is F5. So the detector is asked where the sound sits and the
/// answer is moved to the nearest note of the named class, which corrects the detector's whole-note
/// mistakes while keeping its register.
///
/// With no detection to lean on — silence, a drum, a chord — the class is placed in the octave
/// nearest middle C, which is the same neutral answer an import reaches without a name at all.
#[must_use]
pub fn root_from_key(tag: KeyTag, heard: Option<f32>) -> f32 {
    if let Some(octave) = tag.octave {
        // A name that carries an octave is already a note, and beats anything heard.
        return (f32::from(tag.class) + (octave + 1) as f32 * 12.0).clamp(0.0, 127.0);
    }
    let anchor = heard.filter(|note| note.is_finite()).unwrap_or(60.0);
    nearest_of_class(tag.class, anchor)
}

/// The note of `class` closest to `anchor`.
fn nearest_of_class(class: u8, anchor: f32) -> f32 {
    let class = f32::from(class % 12);
    // The octave `anchor` sits in, then its neighbours: the nearest instance can be either side.
    let base = ((anchor - class) / 12.0).round() * 12.0 + class;
    [base - 12.0, base, base + 12.0]
        .into_iter()
        .filter(|note| (0.0..=127.0).contains(note))
        .min_by(|a, b| {
            (a - anchor)
                .abs()
                .partial_cmp(&(b - anchor).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_of(stem: &str) -> Option<u8> {
        key_in_name(stem).map(|tag| tag.class)
    }

    /// Every shape the library actually uses, with the counts from the survey behind each.
    #[test]
    fn the_shapes_the_library_uses_are_read() {
        // Bare letters — 338 of them, and the commonest shape.
        assert_eq!(class_of("OS_VV2_808_synth_bounce_A"), Some(9));
        assert_eq!(class_of("OS_VV2_electric_bass_guitar_octave_C"), Some(0));
        // Accidentals, both spellings.
        assert_eq!(class_of("OS_VV2_electric_bass_guitar_drop_D#"), Some(3));
        assert_eq!(class_of("pad_Eb"), Some(3));
        assert_eq!(class_of("pad_Bb"), Some(10));
        // Modes, which are not pitches.
        assert_eq!(class_of("jh_keys_piano_smoke_80_Cm"), Some(0));
        assert_eq!(class_of("loop_Am"), Some(9));
        assert_eq!(class_of("loop_G#m"), Some(8));
        assert_eq!(class_of("loop_C#min"), Some(1));
        assert_eq!(class_of("loop_Ebmaj"), Some(3));
        // An octave, when it is offered.
        assert_eq!(
            key_in_name("hit_C3"),
            Some(KeyTag {
                class: 0,
                octave: Some(3)
            })
        );
    }

    /// **The false positives are the whole risk**, because a wrong key retunes the keyboard.
    #[test]
    fn what_is_not_a_key_is_not_read_as_one() {
        // No lowercase last token exists in the library, and a take marker is exactly what one
        // would be. Reading `_b` as B flat-out breaks a drum folder.
        assert_eq!(class_of("kick_loop_b"), None);
        assert_eq!(class_of("take_a"), None);
        // A word that starts with a note letter is not a key.
        assert_eq!(class_of("stem_Bass"), None);
        assert_eq!(class_of("vocal_Chop"), None);
        assert_eq!(class_of("drums_Fill"), None);
        // Letters outside A-G.
        assert_eq!(class_of("synth_H"), None);
        assert_eq!(class_of("riser_X"), None);
        // The key is the last token, not any token: a name that mentions one mid-way is a guess.
        assert_eq!(class_of("Cm_piano_smoke"), None);
        assert_eq!(class_of(""), None);
    }

    /// The name gives the class and the audio gives the octave.
    #[test]
    fn the_octave_comes_from_what_was_heard() {
        let f = key_in_name("bass_F").expect("F");
        // A detector that heard F#2 is corrected to the F beside it, not moved octaves.
        assert_eq!(root_from_key(f, Some(42.3)), 41.0);
        // One that heard a whole note out in the other direction, likewise.
        assert_eq!(root_from_key(f, Some(40.1)), 41.0);
        // Two semitones out — the `soul_E` case from the real pack, where D2 was heard and E is
        // claimed — moves to the E beside it rather than to any other E.
        let e = key_in_name("bass_E").expect("E");
        assert_eq!(root_from_key(e, Some(38.1)), 40.0);
    }

    /// A named octave beats the audio, and nothing heard still gives a usable note.
    #[test]
    fn a_named_octave_wins_and_silence_still_answers() {
        let c3 = key_in_name("hit_C3").expect("C3");
        assert_eq!(root_from_key(c3, Some(72.0)), 48.0, "the name carried C3");
        let g = key_in_name("noise_G").expect("G");
        let placed = root_from_key(g, None);
        assert_eq!(placed % 12.0, 7.0, "G, wherever it was put");
        assert!(
            (placed - 60.0).abs() <= 6.0,
            "with nothing heard it belongs near middle C, not at {placed}"
        );
    }
}
