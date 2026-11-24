mod usb_host_model;

use device::rate_change;
use device::usb::endpoint::{BufferSlot, Event, InEntry, ListEntry, OutEntry};
use device::usb::function::{self, Bus, Configuration};
use device::usb::request::{Address, DataTerminal};
use device::usb::service::Notice;
use device::usb::{
    BENCH_DEVICE_DESCRIPTOR, BENCH_STRINGS, BusSpeed, DEVICE_QUALIFIER, FACTORY_SERIAL_TEMPLATE,
    configuration,
};
use dsp::SampleRate;
use stream::FeedbackValue;
use usb_host_model::{Bench, Stalled, with_bench};

const fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
    let [vl, vh] = value.to_le_bytes();
    let [il, ih] = index.to_le_bytes();
    let [ll, lh] = length.to_le_bytes();
    [request_type, request, vl, vh, il, ih, ll, lh]
}

const fn get_descriptor(kind: u8, index: u8, language: u16, length: u16) -> [u8; 8] {
    setup(
        0x80,
        0x06,
        (kind as u16) << 8 | index as u16,
        language,
        length,
    )
}

const CLOCK_ON_AUDIO_CONTROL: u16 = 0x0A00;
const GET_RANGE_HEAD: [u8; 8] = setup(0xA1, 0x02, 0x0100, CLOCK_ON_AUDIO_CONTROL, 2);
const GET_RANGE: [u8; 8] = setup(0xA1, 0x02, 0x0100, CLOCK_ON_AUDIO_CONTROL, 74);
const GET_CUR: [u8; 8] = setup(0xA1, 0x01, 0x0100, CLOCK_ON_AUDIO_CONTROL, 4);
const SET_CUR: [u8; 8] = setup(0x21, 0x01, 0x0100, CLOCK_ON_AUDIO_CONTROL, 4);
const GET_CLOCK_VALIDITY: [u8; 8] = setup(0xA1, 0x01, 0x0200, CLOCK_ON_AUDIO_CONTROL, 1);
const SET_LINE_CODING: [u8; 8] = setup(0x21, 0x20, 0, 3, 7);
const GET_LINE_CODING: [u8; 8] = setup(0xA1, 0x21, 0, 3, 7);
const SET_CONTROL_LINE_STATE_PRESENT: [u8; 8] = setup(0x21, 0x22, 1, 3, 0);
const SET_CONFIGURATION_1: [u8; 8] = setup(0x00, 0x09, 1, 0, 0);
const GET_CONFIGURATION: [u8; 8] = setup(0x80, 0x08, 0, 0, 1);
const VENDOR_OUT_Q: [u8; 8] = setup(0x40, b'q', 0, 7, 0);
const LINE_CODING_9600_8N1: [u8; 7] = [0x80, 0x25, 0x00, 0x00, 0x00, 0x00, 0x08];

fn set_address(address: u8) -> [u8; 8] {
    setup(0x00, 0x05, u16::from(address), 0, 0)
}

fn set_interface(interface: u16, setting: u16) -> [u8; 8] {
    setup(0x01, 0x0B, setting, interface, 0)
}

fn address_value(address: Address) -> u8 {
    match address {
        Address::Default => 0,
        Address::Assigned(assigned) => assigned.value(),
    }
}

fn enumerate_like_windows(bench: &mut Bench<'_>, address: u8) {
    bench.attach_and_reset(BusSpeed::High);
    let first_look = bench.control_read(get_descriptor(1, 0, 0, 64)).unwrap();
    assert_eq!(
        first_look, BENCH_DEVICE_DESCRIPTOR,
        "a short reply ends the 64-byte read"
    );
    bench.deliver(Event::BusReset(BusSpeed::High));
    assert_eq!(bench.address, Address::Default);

    bench.control_write(set_address(address), &[]).unwrap();
    assert_eq!(address_value(bench.address), address);

    assert_eq!(
        bench.control_read(get_descriptor(1, 0, 0, 18)).unwrap(),
        BENCH_DEVICE_DESCRIPTOR
    );
    let header = bench.control_read(get_descriptor(2, 0, 0, 9)).unwrap();
    let total = u16::from_le_bytes([header[2], header[3]]);
    let whole = bench.control_read(get_descriptor(2, 0, 0, total)).unwrap();
    assert_eq!(whole, configuration(BusSpeed::High));
    assert_eq!(
        bench.control_read(get_descriptor(6, 0, 0, 10)).unwrap(),
        DEVICE_QUALIFIER
    );
    assert_eq!(
        bench.control_read(get_descriptor(3, 0, 0, 255)).unwrap(),
        [4, 3, 0x09, 0x04]
    );
    assert_eq!(
        bench
            .control_read(get_descriptor(3, 2, 0x0409, 255))
            .unwrap(),
        BENCH_STRINGS.product.descriptor()
    );
    assert_eq!(
        bench
            .control_read(get_descriptor(3, 3, 0x0409, 255))
            .unwrap(),
        FACTORY_SERIAL_TEMPLATE.descriptor()
    );
    assert_eq!(
        bench.control_read(get_descriptor(3, 0xEE, 0, 18)),
        Err(Stalled),
        "string 0xEE stalls"
    );

    assert_eq!(bench.control_read(GET_CONFIGURATION).unwrap(), [0]);
    bench.control_write(SET_CONFIGURATION_1, &[]).unwrap();
    assert_eq!(bench.control_read(GET_CONFIGURATION).unwrap(), [1]);
    assert!(
        bench.is_armed(ListEntry::Out(OutEntry::Serial)),
        "the console OUT endpoint listens once configured"
    );

    let head = bench.control_read(GET_RANGE_HEAD).unwrap();
    assert_eq!(head, [6, 0]);
    let range = bench.control_read(GET_RANGE).unwrap();
    let rates: Vec<u32> = range[2..]
        .chunks(12)
        .map(|triplet| u32::from_le_bytes(triplet[..4].try_into().unwrap()))
        .collect();
    assert_eq!(rates, [44_100, 48_000, 88_200, 96_000, 176_400, 192_000]);
    assert_eq!(bench.control_read(GET_CLOCK_VALIDITY).unwrap(), [1]);
}

fn open_console(bench: &mut Bench<'_>) {
    bench
        .control_write(SET_LINE_CODING, &LINE_CODING_9600_8N1)
        .unwrap();
    assert_eq!(
        bench.control_read(GET_LINE_CODING).unwrap(),
        LINE_CODING_9600_8N1
    );
    bench.notices.clear();
    bench
        .control_write(SET_CONTROL_LINE_STATE_PRESENT, &[])
        .unwrap();
    assert_eq!(bench.notices, [Notice::DataTerminal(DataTerminal::Present)]);
}

fn set_rate(bench: &mut Bench<'_>, rate: SampleRate) {
    bench.notices.clear();
    bench
        .control_write(SET_CUR, &rate.hz().to_le_bytes())
        .unwrap();
    assert_eq!(
        bench.notices,
        [Notice::RateChange(rate_change::Event::SetCur(rate))]
    );
    bench.change_rate_as_the_rate_machine_would(rate);
    assert_eq!(
        bench.control_read(GET_CUR).unwrap(),
        rate.hz().to_le_bytes()
    );
}

fn open_streams(bench: &mut Bench<'_>) {
    bench.control_write(set_interface(1, 1), &[]).unwrap();
    bench.control_write(set_interface(2, 1), &[]).unwrap();
    assert!(bench.is_armed(ListEntry::Out(OutEntry::Playback(BufferSlot::First))));
    assert!(bench.is_armed(ListEntry::Out(OutEntry::Playback(BufferSlot::Second))));
}

fn stream(bench: &mut Bench<'_>, microframes: u64, rate: SampleRate) {
    bench.log = Default::default();
    for _ in 0..microframes {
        bench.microframe(true, true);
    }
    let per_microframe = (rate.hz() / 8000) as usize;
    assert_eq!(
        bench.log.playback_packets_lost, 0,
        "every playback packet found a buffer"
    );
    assert!(
        bench.log.playback_frames_sent
            >= microframes * per_microframe as u64 - per_microframe as u64,
        "the host sent {} frames",
        bench.log.playback_frames_sent
    );
    assert!(
        bench.log.rendered_nonsilent_frames > (microframes / 2) * per_microframe as u64,
        "the tick played what the host sent"
    );
    let polls = bench.log.feedback.len() as u64;
    assert!(polls >= microframes / 8 - 1, "{polls} feedback reports");
    let low = FeedbackValue::lowest(rate).samples_per_microframe_16_16();
    let high = FeedbackValue::highest(rate).samples_per_microframe_16_16();
    assert!(
        bench
            .log
            .feedback
            .iter()
            .all(|ff| (low..=high).contains(ff)),
        "feedback stays in the clamp window"
    );
    assert!(
        bench.log.capture_polls_unanswered <= 1,
        "capture answers every poll after the first"
    );
    let sizes: Vec<usize> = bench.log.capture_packets.iter().map(Vec::len).collect();
    assert!(
        sizes.iter().all(|bytes| bytes % 8 == 0
            && bytes / 8 + 1 >= per_microframe
            && bytes / 8 <= per_microframe + 1),
        "capture packet sizes stay within one frame of nominal: {sizes:?}"
    );
    let samples: Vec<i32> = bench
        .log
        .capture_packets
        .concat()
        .chunks(8)
        .map(|frame| i32::from_le_bytes(frame[..4].try_into().unwrap()) >> 8)
        .filter(|n| *n != 0)
        .collect();
    assert!(samples.len() as u64 > microframes / 2 * per_microframe as u64);
    assert!(
        samples.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "captured frames arrive in order with none lost or repeated"
    );
}

fn configured_audio(
    bench: &Bench<'_>,
) -> Option<(stream::AlternateSetting, stream::AlternateSetting)> {
    match bench.service.function() {
        function::State::Awake(Bus::Configured(Configuration::HighSpeed(audio))) => {
            Some((audio.playback(), audio.capture()))
        }
        _ => None,
    }
}

#[test]
fn a_simulated_host_enumerates_streams_suspends_resets_and_enumerates_again() {
    with_bench(|bench| {
        enumerate_like_windows(bench, 5);
        open_console(bench);
        set_rate(bench, SampleRate::Hz96000);
        open_streams(bench);
        stream(bench, 4_000, SampleRate::Hz96000);

        bench.console.to_host.extend(b"ok\n");
        assert!(bench.send_console_bytes(b"counters\n"));
        assert_eq!(bench.console.from_host, b"counters\n");
        bench.microframe(true, true);
        assert_eq!(bench.read_console_packet().unwrap(), b"ok\n");

        bench.console.from_host_held = true;
        assert!(bench.send_console_bytes(b"status\n"));
        assert!(
            !bench.is_armed(ListEntry::Out(OutEntry::Serial)),
            "a full inbound ring holds the console OUT endpoint instead of dropping bytes"
        );
        assert!(!bench.send_console_bytes(b"dropped?\n"));
        bench.microframe(true, true);
        assert!(!bench.is_armed(ListEntry::Out(OutEntry::Serial)));
        bench.console.from_host_held = false;
        bench.microframe(true, true);
        assert!(
            bench.is_armed(ListEntry::Out(OutEntry::Serial)),
            "the next start of frame re-arms the console once the ring has room"
        );
        assert_eq!(bench.console.from_host, b"counters\nstatus\n");

        bench.notices.clear();
        bench.deliver(Event::Suspended);
        assert_eq!(
            bench.notices,
            [Notice::FadeOutputsToSilence, Notice::SuspendPhantom]
        );
        assert_eq!(
            bench.armed_entries(),
            ["Out(Serial)".to_string()].into_iter().collect(),
            "suspend leaves only the console listening"
        );

        bench.notices.clear();
        bench.deliver(Event::BusReset(BusSpeed::High));
        assert_eq!(
            bench.notices,
            [Notice::ReleaseOutputSilence, Notice::ResumePhantom]
        );
        assert!(
            bench.armed_entries().is_empty(),
            "a reset leaves nothing armed"
        );
        assert_eq!(bench.address, Address::Default);

        bench.change_rate_as_the_rate_machine_would(SampleRate::Hz48000);
        enumerate_like_windows(bench, 9);
        assert_eq!(
            configured_audio(bench),
            Some((
                stream::AlternateSetting::Zero,
                stream::AlternateSetting::Zero
            ))
        );
        open_streams(bench);
        stream(bench, 2_000, SampleRate::Hz48000);
    });
}

#[test]
fn resume_restarts_both_streams_with_their_buffers_armed_again() {
    with_bench(|bench| {
        enumerate_like_windows(bench, 3);
        open_streams(bench);
        stream(bench, 200, SampleRate::Hz48000);
        bench.deliver(Event::Suspended);
        bench.notices.clear();
        bench.deliver(Event::Resumed);
        assert_eq!(
            bench.notices,
            [Notice::ReleaseOutputSilence, Notice::ResumePhantom]
        );
        assert!(bench.is_armed(ListEntry::Out(OutEntry::Playback(BufferSlot::First))));
        stream(bench, 400, SampleRate::Hz48000);
    });
}

#[test]
fn closing_a_stream_disarms_its_endpoints_and_the_other_stream_carries_on() {
    with_bench(|bench| {
        enumerate_like_windows(bench, 4);
        open_streams(bench);
        stream(bench, 100, SampleRate::Hz48000);
        bench.control_write(set_interface(1, 0), &[]).unwrap();
        for entry in [
            ListEntry::Out(OutEntry::Playback(BufferSlot::First)),
            ListEntry::Out(OutEntry::Playback(BufferSlot::Second)),
            ListEntry::In(InEntry::Feedback),
        ] {
            assert!(!bench.is_armed(entry), "{entry:?} stays armed");
        }
        bench.log = Default::default();
        for _ in 0..100 {
            bench.microframe(false, true);
        }
        assert_eq!(bench.log.capture_polls_unanswered, 0);
        assert_eq!(bench.log.capture_packets.len(), 100);
    });
}

#[test]
fn the_vendor_request_that_erases_the_factory_factory_page_stalls_and_changes_nothing() {
    with_bench(|bench| {
        enumerate_like_windows(bench, 6);
        let before = bench.service.function();
        bench.notices.clear();
        assert_eq!(bench.control_write(VENDOR_OUT_Q, &[]), Err(Stalled));
        assert!(bench.notices.is_empty());
        assert_eq!(bench.service.function(), before);
        assert_eq!(
            bench.control_read(get_descriptor(1, 0, 0, 18)).unwrap(),
            BENCH_DEVICE_DESCRIPTOR,
            "the next setup clears the stall"
        );
    });
}

#[test]
fn a_full_speed_attach_offers_only_the_console() {
    with_bench(|bench| {
        bench.attach_and_reset(BusSpeed::Full);
        bench.control_write(set_address(2), &[]).unwrap();
        let header = bench.control_read(get_descriptor(2, 0, 0, 9)).unwrap();
        let total = u16::from_le_bytes([header[2], header[3]]);
        assert_eq!(
            bench.control_read(get_descriptor(2, 0, 0, total)).unwrap(),
            configuration(BusSpeed::Full)
        );
        bench.control_write(SET_CONFIGURATION_1, &[]).unwrap();
        assert_eq!(
            bench.control_write(set_interface(1, 1), &[]),
            Err(Stalled),
            "interface 1 is the console data interface at full speed"
        );
        assert_eq!(bench.control_read(GET_CUR), Err(Stalled));
        assert!(!bench.is_armed(ListEntry::Out(OutEntry::Playback(BufferSlot::First))));
        assert!(bench.send_console_bytes(&[b'x'; 64]));
        assert_eq!(bench.console.from_host, [b'x'; 64]);
    });
}

#[test]
fn a_setup_that_interrupts_a_reply_is_answered_with_fresh_bytes() {
    with_bench(|bench| {
        enumerate_like_windows(bench, 8);
        bench.deliver(Event::Setup(get_descriptor(1, 0, 0, 18)));
        assert!(bench.is_armed(ListEntry::In(InEntry::Control)));
        let header = bench.control_read(get_descriptor(2, 0, 0, 9)).unwrap();
        assert_eq!(header, configuration(BusSpeed::High)[..9]);
    });
}

fn captured_while_streaming(completions_ride_on_the_next_start_of_frame: bool) -> Vec<Vec<u8>> {
    let mut captured = None;
    with_bench(|bench| {
        bench.completions_ride_on_the_next_start_of_frame =
            completions_ride_on_the_next_start_of_frame;
        enumerate_like_windows(bench, 5);
        set_rate(bench, SampleRate::Hz96000);
        open_streams(bench);
        stream(bench, 2_000, SampleRate::Hz96000);
        captured = Some(bench.log.capture_packets.clone());
    });
    captured.expect("the bench ran")
}

#[test]
fn stream_completions_carried_by_the_next_start_of_frame_stream_whole_and_capture_the_same_packets()
{
    assert_eq!(
        captured_while_streaming(true),
        captured_while_streaming(false)
    );
}
