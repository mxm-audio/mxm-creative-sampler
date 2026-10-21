//! How a parameter travels: plain values to the host's normalised `0..=1` and back.
//!
//! **nice-plug's laws, reproduced to the bit**, because they decide what a stored value is: a host
//! sets a normalised value and the plugin keeps `unnormalize` of it, so a phone that rounded
//! differently would store a different number for the same gesture. Ported from nice-plug-core
//! 0.4.2's `src/params/range.rs` (ISC, Copyright (c) 2022-2024 Robbert van der Helm), the
//! continuous ranges and the step snap this plugin uses. The plugin's tests hold every parameter's
//! travel to its nice-plug parameter's.

/// A continuous parameter's travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Range {
    /// Straight from `min` to `max`.
    Linear { min: f32, max: f32 },
    /// `normalized = proportion ^ factor`: a factor below one gives the low end more travel.
    Skewed { min: f32, max: f32, factor: f32 },
    /// Skewed either side of `center`, which sits at the middle of the travel.
    SymmetricalSkewed {
        min: f32,
        max: f32,
        factor: f32,
        center: f32,
    },
}

/// A skew factor as nice-plug's `FloatRange::skew_factor` writes it: `2^factor`, so `-1.0` gives
/// the low half of the range about two thirds of the travel.
#[must_use]
pub fn skew_factor(factor: f32) -> f32 {
    2.0f32.powf(factor)
}

impl Range {
    /// The lowest and highest plain values.
    #[must_use]
    pub fn bounds(&self) -> (f32, f32) {
        match *self {
            Range::Linear { min, max }
            | Range::Skewed { min, max, .. }
            | Range::SymmetricalSkewed { min, max, .. } => (min, max),
        }
    }

    /// A plain value as the host's `0..=1`, the plain value clamped first.
    #[must_use]
    pub fn normalize(&self, plain: f32) -> f32 {
        match *self {
            Range::Linear { min, max } => (plain.clamp(min, max) - min) / (max - min),
            Range::Skewed { min, max, factor } => {
                ((plain.clamp(min, max) - min) / (max - min)).powf(factor)
            }
            Range::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                let unscaled_proportion = (plain.clamp(min, max) - min) / (max - min);
                let center_proportion = (center - min) / (max - min);
                if unscaled_proportion > center_proportion {
                    let scaled_proportion = (unscaled_proportion - center_proportion)
                        * (1.0 - center_proportion).recip();
                    (scaled_proportion.powf(factor) * 0.5) + 0.5
                } else {
                    let inverted_scaled_proportion =
                        (center_proportion - unscaled_proportion) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(factor)) * 0.5
                }
            }
        }
    }

    /// The host's `0..=1` as a plain value, the normalised value clamped first.
    #[must_use]
    pub fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match *self {
            Range::Linear { min, max } => (normalized * (max - min)) + min,
            Range::Skewed { min, max, factor } => {
                (normalized.powf(factor.recip()) * (max - min)) + min
            }
            Range::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                let center_proportion = (center - min) / (max - min);
                let skewed_proportion = if normalized > 0.5 {
                    let scaled_proportion = (normalized - 0.5) * 2.0;
                    (scaled_proportion.powf(factor.recip()) * (1.0 - center_proportion))
                        + center_proportion
                } else {
                    let inverted_scaled_proportion = (0.5 - normalized) * 2.0;
                    (1.0 - inverted_scaled_proportion.powf(factor.recip())) * center_proportion
                };

                (skewed_proportion * (max - min)) + min
            }
        }
    }

    /// `value` on the nearest multiple of `step_size`, within the range.
    #[must_use]
    pub fn snap_to_step(&self, value: f32, step_size: f32) -> f32 {
        let (min, max) = self.bounds();
        ((value / step_size).round() * step_size).clamp(min, max)
    }
}

/// A choice's index as the host's `0..=1`: nice-plug's `IntRange::Linear` from zero, clamped.
#[must_use]
pub fn index_normalized(index: f32, variants: usize) -> f32 {
    let last = variants.saturating_sub(1).max(1);
    ((index as i32) as f32 / last as f32).clamp(0.0, 1.0)
}

/// The host's `0..=1` as a choice's index, rounded to the nearest.
#[must_use]
pub fn index_plain(normalized: f32, variants: usize) -> usize {
    let last = variants.saturating_sub(1);
    (normalized.clamp(0.0, 1.0) * last as f32).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// nice-plug-core's own range tests, on this port.
    #[test]
    fn the_port_keeps_nice_plugs_own_cases() {
        let linear = Range::Linear {
            min: 10.0,
            max: 20.0,
        };
        assert_eq!(linear.normalize(17.5), 0.75);
        assert_eq!(linear.unnormalize(0.25), 12.5);
        assert_eq!(linear.snap_to_step(13.0, 4.73), 14.190001);
        assert_eq!(linear.snap_to_step(10.0, 4.73), 10.0);
        assert_eq!(linear.snap_to_step(20.0, 6.73), 20.0);
        let skewed = Range::Skewed {
            min: 10.0,
            max: 20.0,
            factor: skew_factor(-2.0),
        };
        assert_eq!(skewed.normalize(17.5), 0.9306049);
        assert_eq!(skewed.unnormalize(0.9306049), 17.5);
        assert_eq!(index_normalized(-5.0 + 10.0, 21), 0.25);
        assert_eq!(index_plain(0.73, 21), 15);
    }
}
