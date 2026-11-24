use dsp::{
    Band, BandCoefficients, BandFrequency, BandGain, BandQ, FilterKind, PeqPreset, PresetBand,
    SampleRate,
};

use crate::digest::Fnv1a;

pub const FILTER_KINDS: [(&str, FilterKind); 5] = [
    ("peaking", FilterKind::Peaking),
    ("low-shelf", FilterKind::LowShelf),
    ("high-shelf", FilterKind::HighShelf),
    ("high-pass", FilterKind::HighPass),
    ("low-pass", FilterKind::LowPass),
];

const FIVE_BAND_PLAN: [(FilterKind, f64, f64); 5] = [
    (FilterKind::LowShelf, 80.0, 4.0),
    (FilterKind::Peaking, 250.0, -3.0),
    (FilterKind::Peaking, 1_000.0, 6.0),
    (FilterKind::Peaking, 4_000.0, -6.0),
    (FilterKind::HighShelf, 10_000.0, 3.0),
];

const GRID_HZ: [f64; 8] = [
    20.0, 31.5, 100.0, 440.0, 1_000.0, 3_150.0, 10_000.0, 20_000.0,
];
const GRID_DB: [f64; 5] = [-12.0, -3.0, 0.0, 6.0, 12.0];
const GRID_Q: [f64; 5] = [0.3, core::f64::consts::FRAC_1_SQRT_2, 1.0, 4.0, 10.0];
const SKIPPED_BAND_MARKER: i32 = -1;

fn band(kind: FilterKind, hz: f64, db: f64, q: f64) -> Band {
    Band {
        kind,
        frequency: BandFrequency::from_hz(hz).expect("grid frequency is audible"),
        gain: BandGain::from_db(db).expect("grid gain is within 12 dB"),
        q: BandQ::new(q).expect("grid Q is supported"),
    }
}

pub fn five_band_preset() -> PeqPreset {
    let mut preset = PeqPreset::FLAT;
    for (slot, (kind, hz, db)) in preset.bands.iter_mut().zip(FIVE_BAND_PLAN) {
        *slot = PresetBand {
            band: band(kind, hz, db, 1.0),
            enabled: true,
        };
    }
    preset
}

pub fn design_grid_digest(rate: SampleRate, kind: FilterKind) -> u32 {
    let mut digest = Fnv1a::new();
    for hz in GRID_HZ {
        for db in GRID_DB {
            for q in GRID_Q {
                match BandCoefficients::design(&band(kind, hz, db, q), rate) {
                    BandCoefficients::Skip => digest.absorb(SKIPPED_BAND_MARKER),
                    BandCoefficients::Filter(biquad) => {
                        digest.absorb(biquad.format().post_shift() as i32);
                        for word in biquad.feedforward().into_iter().chain(biquad.feedback()) {
                            digest.absorb(word);
                        }
                    }
                }
            }
        }
    }
    digest.value()
}
