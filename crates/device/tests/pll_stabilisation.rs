use device::lcd::panel::{Hertz, Microseconds};
use device::phantom::Milliseconds;
use device::pll::{
    AUDIO_PLL_STABILISATION_FOR_44K1_FAMILY, AUDIO_PLL_STABILISATION_FOR_48K_FAMILY,
    CORE_PLL_PREDIVIDER, CORE_PLL_STABILISATION, audio_pll_predivider, audio_pll_stabilisation,
    reference_clock, stabilisation_time,
};
use dsp::RateFamily;

#[test]
fn the_audio_pll_reference_is_192_khz_in_both_families() {
    for family in [
        RateFamily::FortyEightKilohertz,
        RateFamily::FortyFourPointOneKilohertz,
    ] {
        assert_eq!(
            reference_clock(audio_pll_predivider(family)),
            Hertz(192_000)
        );
    }
}

#[test]
fn the_core_pll_reference_is_8_mhz() {
    assert_eq!(reference_clock(CORE_PLL_PREDIVIDER), Hertz(8_000_000));
}

#[test]
fn the_errata_wait_matches_the_figures_of_2_58_ms_and_550_us() {
    assert_eq!(stabilisation_time(Hertz(192_000)), Microseconds(2_584));
    assert_eq!(stabilisation_time(Hertz(8_000_000)), Microseconds(550));
}

#[test]
fn each_wait_rounds_up_to_whole_milliseconds_so_it_is_never_shorter_than_the_errata() {
    assert_eq!(AUDIO_PLL_STABILISATION_FOR_48K_FAMILY, Milliseconds(3));
    assert_eq!(AUDIO_PLL_STABILISATION_FOR_44K1_FAMILY, Milliseconds(3));
    assert_eq!(CORE_PLL_STABILISATION, Milliseconds(1));
    assert_eq!(
        audio_pll_stabilisation(RateFamily::FortyEightKilohertz),
        AUDIO_PLL_STABILISATION_FOR_48K_FAMILY
    );
    assert_eq!(
        audio_pll_stabilisation(RateFamily::FortyFourPointOneKilohertz),
        AUDIO_PLL_STABILISATION_FOR_44K1_FAMILY
    );
}
