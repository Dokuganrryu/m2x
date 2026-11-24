mod usb_support;

use device::rate_change::{self, transition};
use device::usb::BusSpeed;
use device::usb::answer::Effect;
use device::usb::function::{Event, State};
use device::usb::request::{BitsPerSecond, DataBits, DataTerminal, LineCoding, Parity, StopBits};
use dsp::SampleRate;
use usb_support::{
    STEADY_48K, assigned, context, context_with_rate, effect, enumerated_at, reply, run, setup,
    stalls,
};

const GET: u8 = 0xA1;
const SET: u8 = 0x21;
const CUR: u8 = 0x01;
const RANGE: u8 = 0x02;
const SAMPLING_FREQUENCY: u16 = 0x0100;
const CLOCK_VALIDITY: u16 = 0x0200;
const CLOCK_ON_AUDIO_CONTROL: u16 = 0x0A00;
const SERIAL_CONTROL_AT_HIGH_SPEED: u16 = 3;
const SERIAL_CONTROL_AT_FULL_SPEED: u16 = 0;

fn high_speed() -> State {
    enumerated_at(BusSpeed::High)
}

fn set_cur(hertz: u32) -> Effect {
    effect(
        setup(SET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
        &hertz.to_le_bytes(),
        &context(high_speed()),
    )
}

fn snapped(hertz: u32) -> SampleRate {
    match set_cur(hertz) {
        Effect::RateChange(rate_change::Event::SetCur(rate)) => rate,
        other => panic!("SET CUR {hertz} gave {other:?}"),
    }
}

fn get_cur_after(set: Effect) -> u32 {
    let Effect::RateChange(event) = set else {
        panic!("{set:?} is not a rate event");
    };
    let (rate, _) = transition(STEADY_48K, event);
    let bytes = reply(
        setup(GET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
        &context_with_rate(high_speed(), rate),
    );
    u32::from_le_bytes(bytes.try_into().unwrap())
}

#[test]
fn set_cur_then_get_cur_returns_each_supported_rate() {
    for hertz in [44_100, 48_000, 88_200, 96_000, 176_400, 192_000] {
        assert_eq!(get_cur_after(set_cur(hertz)), hertz, "{hertz} Hz");
    }
}

#[test]
fn an_off_table_set_cur_snaps_to_the_closest_rate_which_get_cur_then_returns() {
    for (asked, answered) in [
        (0, 44_100),
        (1, 44_100),
        (46_049, 44_100),
        (46_051, 48_000),
        (50_000, 48_000),
        (68_101, 88_200),
        (100_000, 96_000),
        (130_000, 96_000),
        (140_000, 176_400),
        (184_201, 192_000),
        (384_000, 192_000),
        (u32::MAX, 192_000),
    ] {
        assert_eq!(snapped(asked).hz(), answered, "{asked} Hz");
        assert_eq!(
            get_cur_after(set_cur(asked)),
            answered,
            "{asked} Hz read back"
        );
    }
}

#[test]
fn a_set_cur_exactly_between_two_rates_snaps_to_the_lower_rate() {
    for (midpoint, lower) in [
        (46_050, 44_100),
        (68_100, 48_000),
        (92_100, 88_200),
        (136_200, 96_000),
        (184_200, 176_400),
    ] {
        assert_eq!(snapped(midpoint).hz(), lower, "{midpoint} Hz");
    }
}

const SUBRANGE_COUNT: [u8; 2] = [0x06, 0x00];
const SUBRANGES: [[u8; 12]; 6] = [
    [0x44, 0xAC, 0, 0, 0x44, 0xAC, 0, 0, 0, 0, 0, 0],
    [0x80, 0xBB, 0, 0, 0x80, 0xBB, 0, 0, 0, 0, 0, 0],
    [0x88, 0x58, 1, 0, 0x88, 0x58, 1, 0, 0, 0, 0, 0],
    [0x00, 0x77, 1, 0, 0x00, 0x77, 1, 0, 0, 0, 0, 0],
    [0x10, 0xB1, 2, 0, 0x10, 0xB1, 2, 0, 0, 0, 0, 0],
    [0x00, 0xEE, 2, 0, 0x00, 0xEE, 2, 0, 0, 0, 0, 0],
];

fn full_range() -> Vec<u8> {
    [SUBRANGE_COUNT.to_vec(), SUBRANGES.concat()].concat()
}

fn get_range(length: u16) -> Vec<u8> {
    reply(
        setup(
            GET,
            RANGE,
            SAMPLING_FREQUENCY,
            CLOCK_ON_AUDIO_CONTROL,
            length,
        ),
        &context(high_speed()),
    )
}

#[test]
fn get_range_returns_six_single_rate_subranges_in_74_bytes() {
    assert_eq!(get_range(74), full_range());
    assert_eq!(full_range().len(), 74);
}

#[test]
fn get_range_truncates_to_a_shorter_wlength_and_ends_short_for_a_longer_one() {
    assert_eq!(get_range(2), [0x06, 0x00]);
    assert_eq!(get_range(14), full_range()[..14]);
    assert_eq!(get_range(0), Vec::<u8>::new());
    assert_eq!(get_range(255), full_range());
}

#[test]
fn get_cur_truncates_to_a_shorter_wlength() {
    assert_eq!(
        reply(
            setup(GET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 2),
            &context(high_speed())
        ),
        [0x80, 0xBB]
    );
}

#[test]
fn clock_validity_reads_valid_only_while_the_rate_machine_is_steady() {
    let validity = |rate| {
        reply(
            setup(GET, CUR, CLOCK_VALIDITY, CLOCK_ON_AUDIO_CONTROL, 1),
            &context_with_rate(high_speed(), rate),
        )
    };
    assert_eq!(validity(STEADY_48K), [1]);
    let (changing, _) = transition(STEADY_48K, rate_change::Event::SetCur(SampleRate::Hz96000));
    assert_eq!(validity(changing), [0]);
}

#[test]
fn unknown_entities_selectors_channels_and_attributes_stall() {
    let configured = context(high_speed());
    let refused = [
        setup(GET, CUR, 0x0300, CLOCK_ON_AUDIO_CONTROL, 4),
        setup(GET, CUR, 0x0000, CLOCK_ON_AUDIO_CONTROL, 4),
        setup(GET, CUR, 0x0101, CLOCK_ON_AUDIO_CONTROL, 4),
        setup(GET, CUR, 0x0201, CLOCK_ON_AUDIO_CONTROL, 1),
        setup(GET, RANGE, CLOCK_VALIDITY, CLOCK_ON_AUDIO_CONTROL, 14),
        setup(GET, 0x03, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
        setup(GET, 0x00, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0000, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0100, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0300, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0400, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0500, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0B00, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0A01, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0A02, 4),
        setup(GET, CUR, SAMPLING_FREQUENCY, 0x0A05, 4),
        setup(0xA2, CUR, SAMPLING_FREQUENCY, 0x0001, 4),
        setup(0xA2, CUR, SAMPLING_FREQUENCY, 0x0082, 4),
        setup(0xA0, CUR, SAMPLING_FREQUENCY, 0x0000, 4),
        setup(0xA3, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
    ];
    for packet in refused {
        assert!(stalls(packet, &[], &configured), "{packet:02X?}");
    }
}

#[test]
fn writes_the_clock_does_not_support_stall() {
    let configured = context(high_speed());
    let four = 48_000_u32.to_le_bytes();
    for (packet, data) in [
        (
            setup(SET, CUR, CLOCK_VALIDITY, CLOCK_ON_AUDIO_CONTROL, 1),
            &[1][..],
        ),
        (
            setup(SET, RANGE, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
            &four[..],
        ),
        (
            setup(SET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 3),
            &four[..3],
        ),
        (
            setup(SET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 5),
            &[0, 0, 0, 0, 0][..],
        ),
        (
            setup(SET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4),
            &four[..2],
        ),
        (setup(0x22, CUR, SAMPLING_FREQUENCY, 0x0001, 4), &four[..]),
    ] {
        assert!(stalls(packet, data, &configured), "{packet:02X?}");
    }
}

#[test]
fn clock_requests_stall_before_configuration_and_at_full_speed() {
    let packet = setup(GET, CUR, SAMPLING_FREQUENCY, CLOCK_ON_AUDIO_CONTROL, 4);
    let addressed = run(&[
        Event::Attached,
        Event::BusReset(BusSpeed::High),
        Event::AddressSet(assigned(3)),
    ]);
    assert!(stalls(packet, &[], &context(addressed)));
    assert!(stalls(packet, &[], &context(enumerated_at(BusSpeed::Full))));
    assert!(!stalls(packet, &[], &context(high_speed())));
}

fn serial_at(speed: BusSpeed) -> (State, u16) {
    match speed {
        BusSpeed::High => (enumerated_at(speed), SERIAL_CONTROL_AT_HIGH_SPEED),
        BusSpeed::Full => (enumerated_at(speed), SERIAL_CONTROL_AT_FULL_SPEED),
    }
}

#[test]
fn set_line_coding_reports_the_coding_the_host_chose_at_either_speed() {
    let nine_six_hundred_seven_even_one = [0x80, 0x25, 0x00, 0x00, 0x00, 0x02, 0x07];
    for speed in [BusSpeed::High, BusSpeed::Full] {
        let (state, interface) = serial_at(speed);
        assert_eq!(
            effect(
                setup(SET, 0x20, 0, interface, 7),
                &nine_six_hundred_seven_even_one,
                &context(state)
            ),
            Effect::LineCoding(LineCoding {
                rate: BitsPerSecond(9600),
                stop_bits: StopBits::One,
                parity: Parity::Even,
                data_bits: DataBits::Seven,
            }),
            "{speed:?}"
        );
    }
}

#[test]
fn get_line_coding_returns_the_stored_coding_which_starts_at_115200_8n1() {
    for speed in [BusSpeed::High, BusSpeed::Full] {
        let (state, interface) = serial_at(speed);
        assert_eq!(
            reply(setup(GET, 0x21, 0, interface, 7), &context(state)),
            [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08],
            "{speed:?}"
        );
    }
    let mut stored = context(high_speed());
    stored.line_coding = LineCoding {
        rate: BitsPerSecond(921_600),
        stop_bits: StopBits::Two,
        parity: Parity::Odd,
        data_bits: DataBits::Sixteen,
    };
    assert_eq!(
        reply(
            setup(GET, 0x21, 0, SERIAL_CONTROL_AT_HIGH_SPEED, 7),
            &stored
        ),
        [0x00, 0x10, 0x0E, 0x00, 0x02, 0x01, 0x10]
    );
}

#[test]
fn a_line_coding_outside_the_pstn_table_stalls() {
    let (state, interface) = serial_at(BusSpeed::High);
    let configured = context(state);
    for data in [
        [0x80, 0x25, 0, 0, 3, 0, 8],
        [0x80, 0x25, 0, 0, 0, 5, 8],
        [0x80, 0x25, 0, 0, 0, 0, 4],
        [0x80, 0x25, 0, 0, 0, 0, 9],
    ] {
        assert!(
            stalls(setup(SET, 0x20, 0, interface, 7), &data, &configured),
            "{data:02X?}"
        );
    }
    assert!(stalls(
        setup(SET, 0x20, 0, interface, 6),
        &[0x80, 0x25, 0, 0, 0, 0],
        &configured
    ));
    assert!(stalls(
        setup(SET, 0x20, 1, interface, 7),
        &[0x80, 0x25, 0, 0, 0, 0, 8],
        &configured
    ));
}

#[test]
fn set_control_line_state_reports_whether_a_terminal_is_present() {
    let (state, interface) = serial_at(BusSpeed::High);
    let configured = context(state);
    for (bitmap, terminal) in [
        (0x0000, DataTerminal::Absent),
        (0x0001, DataTerminal::Present),
        (0x0002, DataTerminal::Absent),
        (0x0003, DataTerminal::Present),
    ] {
        assert_eq!(
            effect(setup(SET, 0x22, bitmap, interface, 0), &[], &configured),
            Effect::DataTerminal(terminal),
            "{bitmap:#06X}"
        );
    }
    assert!(stalls(
        setup(SET, 0x22, 0x0004, interface, 0),
        &[],
        &configured
    ));
    assert!(stalls(
        setup(SET, 0x22, 0x8001, interface, 0),
        &[],
        &configured
    ));
}

#[test]
fn serial_requests_to_the_wrong_interface_or_unsupported_codes_stall() {
    let (state, _) = serial_at(BusSpeed::High);
    let configured = context(state);
    for packet in [
        setup(GET, 0x21, 0, 4, 7),
        setup(GET, 0x21, 0, 0, 7),
        setup(GET, 0x21, 0, 0x0103, 7),
        setup(SET, 0x23, 0xFFFF, 3, 0),
        setup(SET, 0x02, 0, 3, 0),
        setup(GET, 0x03, 0, 3, 2),
        setup(SET, 0x00, 0, 3, 0),
    ] {
        assert!(stalls(packet, &[], &configured), "{packet:02X?}");
    }
}
