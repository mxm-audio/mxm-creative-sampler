//! **A `Sample` refuses a buffer it cannot reserve instead of aborting its host** (audit D14;
//! `docs/code-review-notes.md` §1).
//!
//! `Sample::new` sizes its analysis buffers from the source, and the plugin builds a `Sample` from
//! restored state, a preset and an import. A bounded source still asks for megabytes, and an
//! infallible allocation that fails aborts the process — a host, in a plugin.
//!
//! **Its own test binary, because it installs a global allocator**: this one refuses allocations
//! of at least [`LARGE`] bytes made on an armed thread. An allocation that is refused and not
//! reserved fallibly aborts this binary, which is what the test looks like when it fails.

use mxm_creative_sampler_dsp::{Sample, SampleError};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Every buffer the probe below sizes from its source is at least this large — the test checks the
/// onset list, the smallest — and nothing the analysis sizes by the rate is: the lag buffers hold 51
/// values at 8 kHz.
const LARGE: usize = 1024;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static SEEN: Cell<usize> = const { Cell::new(0) };
    static REFUSE_AT: Cell<Option<usize>> = const { Cell::new(None) };
    static REFUSED: Cell<bool> = const { Cell::new(false) };
}

/// Counts a large allocation on an armed thread and says whether to refuse it.
fn refuse(size: usize) -> bool {
    if size < LARGE || !ARMED.try_with(Cell::get).unwrap_or(false) {
        return false;
    }
    let index = SEEN.get();
    SEEN.set(index + 1);
    let refused = REFUSE_AT.get() == Some(index);
    if refused {
        REFUSED.set(true);
    }
    refused
}

struct Refusing;

// SAFETY: every call either returns null, which `GlobalAlloc` permits, or forwards to `System`
// with the caller's own arguments.
unsafe impl GlobalAlloc for Refusing {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if refuse(layout.size()) {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if refuse(layout.size()) {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if refuse(new_size) {
            std::ptr::null_mut()
        } else {
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: Refusing = Refusing;

/// A source whose onset map, period track and decimated copy are all large, with attacks in it so
/// the onset list is reserved for real: a 110 Hz tone struck every half second, at 8 kHz.
fn source() -> Vec<[f32; 2]> {
    let rate = 8_000.0f32;
    (0..2_000_000usize)
        .map(|n| {
            let t = n as f32 / rate;
            let since_strike = (n % 4_000) as f32 / rate;
            let value = (std::f32::consts::TAU * 110.0 * t).sin() * (-since_strike * 12.0).exp();
            [value, value * 0.5]
        })
        .collect()
}

#[test]
fn every_source_sized_buffer_in_a_sample_can_be_refused() {
    let frames = source();
    let mut refused_at = 0;
    loop {
        let input = frames.clone();
        SEEN.set(0);
        REFUSE_AT.set(Some(refused_at));
        REFUSED.set(false);
        ARMED.set(true);
        let result = Sample::new(input, 8_000.0);
        ARMED.set(false);
        if !REFUSED.get() {
            let sample = result.expect("with nothing refused, the sample is built");
            assert!(
                std::mem::size_of_val(sample.transients()) >= LARGE,
                "the probe's {} attacks are under the refusal threshold, so refusing the onset list                  was never tried",
                sample.transients().len()
            );
            assert!(
                refused_at >= 4,
                "only {refused_at} source-sized buffers were asked for; the onset map, its onset \
                 list, the decimated copy and the period track are four"
            );
            return;
        }
        assert_eq!(
            result.err(),
            Some(SampleError::Allocation),
            "refusing buffer {refused_at} did not refuse the sample"
        );
        refused_at += 1;
    }
}
