use super::{hsc, lsc, pk};
use dsp::Band;

// WHY: Two original example curves for this project: clarity lifts presence and air and trims low-mid boxiness, warm adds a 105 Hz low shelf and rolls the treble off; each trim is -ceil(peak x 10) / 10 of the worst peak from 20 Hz to 20 kHz over every rate and band set the preset can play (clarity +2.568 dB near 3 kHz, warm +4.126 dB near 53 Hz), so no steady tone exceeds 0 dBFS
pub(super) const CLARITY_TRIM_DB: f64 = -2.6;
pub(super) const WARM_TRIM_DB: f64 = -4.2;

pub(super) const CLARITY: [Band; 14] = [
    lsc(80.0, -1.0, 0.7),
    pk(250.0, -1.5, 1.0),
    pk(3000.0, 1.5, 1.2),
    hsc(10000.0, 1.5, 0.7),
    pk(2000.0, 1.0, 1.0),
    pk(400.0, -1.0, 1.2),
    pk(6000.0, -1.0, 3.0),
    pk(4500.0, 1.0, 1.5),
    pk(120.0, -0.5, 1.0),
    pk(700.0, -0.5, 1.5),
    pk(1200.0, 0.5, 1.2),
    pk(8000.0, 0.5, 1.0),
    pk(14000.0, 0.5, 1.0),
    pk(40.0, -0.5, 0.8),
];

// WHY: The 7 bands with the most effect, played at 88.2 and 96 kHz when the core stays at 100 MHz
pub(super) const CLARITY_HIGH_RATE: [Band; 7] = [
    lsc(80.0, -1.0, 0.7),
    pk(250.0, -1.5, 1.0),
    pk(3000.0, 1.5, 1.2),
    hsc(10000.0, 1.5, 0.7),
    pk(2000.0, 1.0, 1.0),
    pk(400.0, -1.0, 1.2),
    pk(6000.0, -1.0, 3.0),
];

pub(super) const WARM: [Band; 14] = [
    lsc(105.0, 3.0, 0.7),
    hsc(9000.0, -2.0, 0.7),
    pk(5000.0, -1.5, 1.2),
    pk(180.0, 1.0, 0.9),
    pk(2500.0, -1.0, 1.0),
    pk(3500.0, -1.0, 1.5),
    pk(60.0, 1.0, 1.0),
    pk(7000.0, -1.0, 2.0),
    pk(320.0, 0.5, 1.0),
    pk(600.0, -0.5, 1.0),
    pk(1000.0, -0.5, 1.2),
    pk(12000.0, -0.5, 1.0),
    pk(30.0, 0.5, 0.8),
    pk(1600.0, 0.3, 1.0),
];

pub(super) const WARM_HIGH_RATE: [Band; 7] = [
    lsc(105.0, 3.0, 0.7),
    hsc(9000.0, -2.0, 0.7),
    pk(5000.0, -1.5, 1.2),
    pk(180.0, 1.0, 0.9),
    pk(2500.0, -1.0, 1.0),
    pk(3500.0, -1.0, 1.5),
    pk(60.0, 1.0, 1.0),
];

// WHY: On a 150 MHz core all 14 bands play at 88.2 and 96 kHz too; the firmware designs coefficients per rate from the same bands and the worst peak is the one above, so the trims are unchanged
#[cfg(feature = "sound3")]
pub(super) const CLARITY_TRIM_DB_AT_150_MHZ: f64 = -2.6;
#[cfg(feature = "sound3")]
pub(super) const WARM_TRIM_DB_AT_150_MHZ: f64 = -4.2;

#[cfg(feature = "sound3")]
pub(super) const CLARITY_HIGH_RATE_14: [Band; 14] = CLARITY;

#[cfg(feature = "sound3")]
pub(super) const WARM_HIGH_RATE_14: [Band; 14] = WARM;
