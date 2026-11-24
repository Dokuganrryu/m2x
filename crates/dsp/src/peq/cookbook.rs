use core::f64::consts::TAU;

use super::band::{Band, FilterKind};
use super::series::{sine_cosine_from_zero_to_pi, ten_to_the_power_of_a_fraction};
use crate::rate::SampleRate;

pub(crate) struct DesignedBiquad {
    pub(crate) feedforward: [f64; 3],
    pub(crate) feedback: [f64; 2],
}

// WHY: Inlined into both the plain and the trimmed design it cost 1.7 KB of an image region with 592 bytes to spare
#[cfg_attr(feature = "bands14", inline(never))]
pub(crate) fn design(band: &Band, rate: SampleRate) -> DesignedBiquad {
    let angle = TAU * band.frequency.hz() / f64::from(rate.hz());
    let trig = sine_cosine_from_zero_to_pi(angle);
    let c = trig.cosine;
    let alpha = trig.sine / (2.0 * band.q.value());
    let amplitude = ten_to_the_power_of_a_fraction(band.gain.db() / 40.0);
    let shelf_alpha = 2.0 * ten_to_the_power_of_a_fraction(band.gain.db() / 80.0) * alpha;
    let a = amplitude;
    let (numerator, denominator) = match band.kind {
        FilterKind::Peaking => (
            [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a],
        ),
        FilterKind::LowShelf => (
            [
                a * ((a + 1.0) - (a - 1.0) * c + shelf_alpha),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - shelf_alpha),
            ],
            [
                (a + 1.0) + (a - 1.0) * c + shelf_alpha,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - shelf_alpha,
            ],
        ),
        FilterKind::HighShelf => (
            [
                a * ((a + 1.0) + (a - 1.0) * c + shelf_alpha),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - shelf_alpha),
            ],
            [
                (a + 1.0) - (a - 1.0) * c + shelf_alpha,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - shelf_alpha,
            ],
        ),
        FilterKind::HighPass => (
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        ),
        FilterKind::LowPass => (
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        ),
    };
    let leading = denominator[0];
    DesignedBiquad {
        feedforward: numerator.map(|v| v / leading),
        feedback: [-denominator[1] / leading, -denominator[2] / leading],
    }
}
