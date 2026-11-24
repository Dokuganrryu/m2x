#![no_std]

use dsp::{BandCoefficients, CoefficientSet, Generation, PEQ_BANDS, SampleRate};
use golden::{FILTER_KINDS, STANDARD_SCRIPT, design_grid_digest, five_band_preset, standard_run};

const RATES: usize = SampleRate::ALL.len();
const TICKS: usize = STANDARD_SCRIPT.ticks as usize;
const SKIPPED_BAND_POST_SHIFT: i32 = -1;

pub type TickColumns = [[u32; 4]; TICKS];
pub type BandWords = [[[i32; 6]; PEQ_BANDS]; RATES];
pub type GridDigests = [[u32; FILTER_KINDS.len()]; RATES];

fn rate(code: u32) -> SampleRate {
    SampleRate::ALL[code as usize % RATES]
}

#[unsafe(no_mangle)]
pub extern "C" fn golden_ticks(rate_code: u32, out: &mut TickColumns) -> u32 {
    let mut run = standard_run(rate(rate_code));
    let mut written = 0;
    while let Some(words) = run.advance() {
        out[words.tick as usize % TICKS] = words.digest().columns();
        written += 1;
    }
    written
}

#[unsafe(no_mangle)]
pub extern "C" fn golden_bands(out: &mut BandWords) {
    for (rate, bands) in SampleRate::ALL.into_iter().zip(out.iter_mut()) {
        let set = CoefficientSet::design(&five_band_preset(), rate, Generation::FIRST);
        for (designed, words) in set.bands().iter().zip(bands.iter_mut()) {
            *words = match designed {
                BandCoefficients::Skip => [SKIPPED_BAND_POST_SHIFT, 0, 0, 0, 0, 0],
                BandCoefficients::Filter(biquad) => {
                    let [b0, b1, b2] = biquad.feedforward();
                    let [f1, f2] = biquad.feedback();
                    [biquad.format().post_shift() as i32, b0, b1, b2, f1, f2]
                }
            };
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn golden_grid(out: &mut GridDigests) {
    for (rate, digests) in SampleRate::ALL.into_iter().zip(out.iter_mut()) {
        for ((_, kind), digest) in FILTER_KINDS.into_iter().zip(digests.iter_mut()) {
            *digest = design_grid_digest(rate, kind);
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
