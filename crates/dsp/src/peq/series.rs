use core::f64::consts::{FRAC_PI_2, LN_10, PI};

const SINE_COSINE_TERMS: u32 = 12;
const EXPONENTIAL_TERMS: u32 = 24;

pub(crate) struct SineCosine {
    pub(crate) sine: f64,
    pub(crate) cosine: f64,
}

pub(crate) fn sine_cosine_from_zero_to_pi(angle: f64) -> SineCosine {
    if angle > FRAC_PI_2 {
        let reflected = sine_cosine_up_to_half_pi(PI - angle);
        SineCosine {
            sine: reflected.sine,
            cosine: -reflected.cosine,
        }
    } else {
        sine_cosine_up_to_half_pi(angle)
    }
}

fn sine_cosine_up_to_half_pi(angle: f64) -> SineCosine {
    let squared = angle * angle;
    let mut sine_term = angle;
    let mut cosine_term = 1.0;
    let mut sine = sine_term;
    let mut cosine = cosine_term;
    for n in 1..=SINE_COSINE_TERMS {
        let even = f64::from(2 * n);
        sine_term *= -squared / (even * (even + 1.0));
        cosine_term *= -squared / ((even - 1.0) * even);
        sine += sine_term;
        cosine += cosine_term;
    }
    SineCosine { sine, cosine }
}

pub(crate) fn ten_to_the_power_of_a_fraction(exponent: f64) -> f64 {
    exponential_of_a_fraction(exponent * LN_10)
}

fn exponential_of_a_fraction(power: f64) -> f64 {
    (1..=EXPONENTIAL_TERMS)
        .rev()
        .fold(1.0, |tail, n| 1.0 + power / f64::from(n) * tail)
}

#[cfg(test)]
mod accuracy {
    extern crate std;

    use super::{sine_cosine_from_zero_to_pi, ten_to_the_power_of_a_fraction};

    const STEPS: u32 = 200_000;

    #[test]
    fn sine_and_cosine_stay_within_two_epsilon_of_the_host_library_from_zero_to_pi() {
        let mut worst = 0.0f64;
        for step in 0..=STEPS {
            let angle = core::f64::consts::PI * f64::from(step) / f64::from(STEPS);
            let ours = sine_cosine_from_zero_to_pi(angle);
            worst = worst
                .max((ours.sine - angle.sin()).abs())
                .max((ours.cosine - angle.cos()).abs());
        }
        assert!(
            worst <= 2.0 * f64::EPSILON,
            "worst absolute error {worst:e}"
        );
    }

    #[test]
    fn powers_of_ten_stay_within_two_epsilon_of_the_host_library_for_every_band_gain() {
        let mut worst = 0.0f64;
        for step in 0..=STEPS {
            let exponent = -0.3 + 0.6 * f64::from(step) / f64::from(STEPS);
            let host = 10f64.powf(exponent);
            let relative = (ten_to_the_power_of_a_fraction(exponent) - host).abs() / host;
            worst = worst.max(relative);
        }
        assert!(
            worst <= 2.0 * f64::EPSILON,
            "worst relative error {worst:e}"
        );
    }
}
