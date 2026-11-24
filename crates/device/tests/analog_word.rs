use device::analog_word::{AnalogWord, PreampCode, SHIFT_BYTE};
use device::phantom::PhantomGrant;
use dsp::FramesPerTick;

fn code(value: u8) -> PreampCode {
    PreampCode::new(value).unwrap()
}

#[test]
fn the_all_off_word_is_zero_in_the_boot_order() {
    assert_eq!(AnalogWord::ALL_OFF.bits(), 0x0000);
}

#[test]
fn the_shift_byte_is_always_zero() {
    assert_eq!(SHIFT_BYTE, 0x00);
}

#[test]
fn a_word_with_minimum_codes_single_speed_and_no_grant_is_the_all_off_word() {
    let word = AnalogWord::new(
        PreampCode::MINIMUM,
        PreampCode::MINIMUM,
        FramesPerTick::Six,
        PhantomGrant::NONE,
    );
    assert_eq!(word, AnalogWord::ALL_OFF);
}

#[test]
fn preamp_one_code_fills_bits_8_to_13() {
    let word = AnalogWord::new(
        code(14),
        PreampCode::MINIMUM,
        FramesPerTick::Six,
        PhantomGrant::NONE,
    );
    assert_eq!(word.bits(), 0x0E00);
}

#[test]
fn preamp_two_code_fills_bits_0_to_5() {
    let word = AnalogWord::new(
        PreampCode::MINIMUM,
        code(14),
        FramesPerTick::Six,
        PhantomGrant::NONE,
    );
    assert_eq!(word.bits(), 0x000E);
}

#[test]
fn double_speed_sets_bit_15() {
    let word = AnalogWord::new(
        PreampCode::MINIMUM,
        PreampCode::MINIMUM,
        FramesPerTick::Twelve,
        PhantomGrant::NONE,
    );
    assert_eq!(word.bits(), 0x8000);
}

#[test]
fn quad_speed_sets_bit_7() {
    let word = AnalogWord::new(
        PreampCode::MINIMUM,
        PreampCode::MINIMUM,
        FramesPerTick::TwentyFour,
        PhantomGrant::NONE,
    );
    assert_eq!(word.bits(), 0x0080);
}

#[test]
fn every_field_lands_without_touching_the_others() {
    let word = AnalogWord::new(code(5), code(9), FramesPerTick::Twelve, PhantomGrant::NONE);
    assert_eq!(word.bits(), 0x8509);
}

#[test]
fn preamp_codes_admit_zero_to_fourteen_only() {
    for value in 0..=14 {
        assert_eq!(PreampCode::new(value).map(PreampCode::value), Some(value));
    }
    for value in 15..=u8::MAX {
        assert_eq!(PreampCode::new(value), None);
    }
    assert_eq!(PreampCode::MAXIMUM.value(), 14);
    assert_eq!(PreampCode::MINIMUM.value(), 0);
}

#[test]
fn no_grant_and_no_code_can_reach_a_phantom_bit() {
    const PHANTOM_BITS: u16 = 0x4040;
    for one in 0..=14 {
        for two in 0..=14 {
            for speed in [
                FramesPerTick::Six,
                FramesPerTick::Twelve,
                FramesPerTick::TwentyFour,
            ] {
                let word = AnalogWord::new(code(one), code(two), speed, PhantomGrant::NONE);
                assert_eq!(word.bits() & PHANTOM_BITS, 0);
            }
        }
    }
}
