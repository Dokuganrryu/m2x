use device::knob::{Knob, KnobChannel, KnobReading, KnobReadings, Knobs};

#[test]
fn adc_channels_0a_1a_2a_3a_and_0b_are_knobs_0_to_4_in_the_factory_roles() {
    assert_eq!(
        Knob::ALL.map(Knob::channel),
        [
            KnobChannel::Channel0A,
            KnobChannel::Channel1A,
            KnobChannel::Channel2A,
            KnobChannel::Channel3A,
            KnobChannel::Channel0B,
        ]
    );
    assert_eq!(
        Knob::ALL,
        [
            Knob::PreampTwo,
            Knob::Main,
            Knob::MonitorMix,
            Knob::PreampOne,
            Knob::Headphone,
        ]
    );
}

#[test]
fn a_result_fifo_tag_names_the_knob_its_command_scanned() {
    assert_eq!(Knob::from_result_tag(0), None);
    assert_eq!(Knob::from_result_tag(1), Some(Knob::PreampTwo));
    assert_eq!(Knob::from_result_tag(2), Some(Knob::Main));
    assert_eq!(Knob::from_result_tag(3), Some(Knob::MonitorMix));
    assert_eq!(Knob::from_result_tag(4), Some(Knob::PreampOne));
    assert_eq!(Knob::from_result_tag(5), Some(Knob::Headphone));
    assert_eq!(Knob::from_result_tag(6), None);
}

fn every_knob_at(raw: u16) -> KnobReadings {
    KnobReadings([KnobReading(raw); 5])
}

fn main_position(knobs: Knobs) -> u8 {
    knobs.position(Knob::Main).value()
}

#[test]
fn the_first_scan_takes_the_high_byte_of_each_reading() {
    let mut readings = every_knob_at(0);
    readings.0[1] = KnobReading(0x6F42);
    readings.0[4] = KnobReading(0x1280);
    let knobs = Knobs::first_scan(readings);
    assert_eq!(main_position(knobs), 0x6F);
    assert_eq!(knobs.position(Knob::Headphone).value(), 0x12);
    assert_eq!(knobs.position(Knob::PreampTwo).value(), 0);
}

#[test]
fn a_reading_one_below_the_cache_with_a_low_byte_above_0x80_keeps_the_cache() {
    let knobs = Knobs::first_scan(every_knob_at(0x5000));
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0x4F81))), 0x50);
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0x4FFF))), 0x50);
}

#[test]
fn a_reading_one_below_the_cache_with_a_low_byte_at_or_below_0x80_moves_the_cache() {
    let knobs = Knobs::first_scan(every_knob_at(0x5000));
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0x4F80))), 0x4F);
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0x4F00))), 0x4F);
}

#[test]
fn readings_above_the_cache_or_two_below_it_move_the_cache_at_once() {
    let knobs = Knobs::first_scan(every_knob_at(0x5000));
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0x5100))), 0x51);
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0x4EFF))), 0x4E);
}

#[test]
fn a_cache_at_zero_has_no_reading_one_below_it() {
    let knobs = Knobs::first_scan(every_knob_at(0x0000));
    assert_eq!(main_position(knobs.after_scan(every_knob_at(0xFFFF))), 240);
}

#[test]
fn the_position_is_the_cache_clamped_to_240() {
    assert_eq!(main_position(Knobs::first_scan(every_knob_at(0xF0FF))), 240);
    assert_eq!(main_position(Knobs::first_scan(every_knob_at(0xFFFF))), 240);
    assert_eq!(main_position(Knobs::first_scan(every_knob_at(0xEFFF))), 239);
}

#[test]
fn a_cache_above_240_still_holds_against_a_reading_one_below_it() {
    let knobs = Knobs::first_scan(every_knob_at(0xF500));
    let held = knobs.after_scan(every_knob_at(0xF4C0));
    let moved = held.after_scan(every_knob_at(0xEF00));
    assert_eq!(held, knobs);
    assert_eq!(main_position(held), 240);
    assert_eq!(main_position(moved), 0xEF);
}
