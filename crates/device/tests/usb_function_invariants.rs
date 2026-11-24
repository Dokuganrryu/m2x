mod usb_support;

use device::usb::BusSpeed;
use device::usb::answer::{Answer, Effect, answer};
use device::usb::function::{Bus, Configuration, Event, Halt, Output, State};
use device::usb::request::{HaltableEndpoint, InterfaceSelection};
use stream::AlternateSetting;
use usb_support::{XorShift, assigned, context, setup, step};

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;
const HALTABLE: [HaltableEndpoint; 3] = [
    HaltableEndpoint::SerialNotification,
    HaltableEndpoint::SerialReceive,
    HaltableEndpoint::SerialTransmit,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Board {
    playback: AlternateSetting,
    capture: AlternateSetting,
    phantom_suspended: bool,
    outputs_silenced: bool,
    halted: [bool; 3],
}

impl Board {
    const POWER_ON: Self = Self {
        playback: AlternateSetting::Zero,
        capture: AlternateSetting::Zero,
        phantom_suspended: false,
        outputs_silenced: false,
        halted: [false; 3],
    };

    fn slot(endpoint: HaltableEndpoint) -> usize {
        HALTABLE.iter().position(|e| *e == endpoint).unwrap()
    }

    fn execute(&mut self, event: Event, outputs: &[Output]) {
        if matches!(event, Event::BusReset(_) | Event::Detached) {
            self.halted = [false; 3];
        }
        for output in outputs {
            match *output {
                Output::SelectPlayback(setting) => self.playback = setting,
                Output::SelectCapture(setting) => self.capture = setting,
                Output::SuspendPhantom => self.phantom_suspended = true,
                Output::ResumePhantom => self.phantom_suspended = false,
                Output::FadeOutputsToSilence => self.outputs_silenced = true,
                Output::ReleaseOutputSilence => self.outputs_silenced = false,
                Output::HaltEndpoint(endpoint) => self.halted[Self::slot(endpoint)] = true,
                Output::ClearEndpointHalt(endpoint) => self.halted[Self::slot(endpoint)] = false,
            }
        }
    }
}

fn configured_audio(state: State) -> Option<(AlternateSetting, AlternateSetting)> {
    match state {
        State::Awake(Bus::Configured(Configuration::HighSpeed(audio))) => {
            Some((audio.playback(), audio.capture()))
        }
        State::Awake(_) | State::Suspended(_) | State::Detached => None,
    }
}

fn configuration(state: State) -> Option<Configuration> {
    match state {
        State::Awake(Bus::Configured(configuration))
        | State::Suspended(Bus::Configured(configuration)) => Some(configuration),
        State::Awake(_) | State::Suspended(_) | State::Detached => None,
    }
}

fn check(seed: u64, at: usize, before: State, event: Event, after: State, board: Board) {
    let context =
        format!("seed {seed} step {at}: {before:?} on {event:?} gave {after:?}, board {board:?}");
    let (playback_allowed, capture_allowed) =
        configured_audio(after).unwrap_or((AlternateSetting::Zero, AlternateSetting::Zero));
    assert_eq!(
        board.playback, playback_allowed,
        "playback runs only when configured at 1: {context}"
    );
    assert_eq!(
        board.capture, capture_allowed,
        "capture runs only when configured at 1: {context}"
    );
    if matches!(event, Event::BusReset(_) | Event::Suspended) && before != State::Detached {
        assert_eq!(board.playback, AlternateSetting::Zero, "{context}");
        assert_eq!(board.capture, AlternateSetting::Zero, "{context}");
    }
    let suspended = matches!(after, State::Suspended(_));
    assert_eq!(
        board.phantom_suspended, suspended,
        "phantom requests are cleared exactly while suspended: {context}"
    );
    assert_eq!(
        board.outputs_silenced, suspended,
        "outputs are silenced exactly while suspended: {context}"
    );
    match configuration(after) {
        Some(configuration) => {
            for endpoint in HALTABLE {
                let halted = configuration.serial().halt(endpoint) == Halt::Set;
                assert_eq!(
                    board.halted[Board::slot(endpoint)],
                    halted,
                    "{endpoint:?} halt: {context}"
                );
            }
        }
        None => assert_eq!(
            board.halted, [false; 3],
            "no halt outlives the configuration: {context}"
        ),
    }
}

fn random_event(random: &mut XorShift) -> Event {
    let alternate = |random: &mut XorShift| {
        if random.below(2) == 0 {
            AlternateSetting::Zero
        } else {
            AlternateSetting::One
        }
    };
    let speed = |random: &mut XorShift| {
        if random.below(4) == 0 {
            BusSpeed::Full
        } else {
            BusSpeed::High
        }
    };
    let endpoint = |random: &mut XorShift| HALTABLE[random.below(3) as usize];
    match random.below(100) {
        0..=2 => Event::Attached,
        3 if random.below(10) == 0 => Event::Detached,
        3..=5 => Event::BusReset(speed(random)),
        6..=8 => Event::Suspended,
        9..=14 => Event::Resumed,
        15..=22 => Event::AddressSet(assigned(random.below(128) as u8)),
        23..=25 => Event::ConfigurationSet,
        26 => Event::ConfigurationCleared,
        27..=58 => Event::InterfaceSelected(InterfaceSelection::Playback(alternate(random))),
        59..=73 => Event::InterfaceSelected(InterfaceSelection::Capture(alternate(random))),
        74..=77 => Event::InterfaceSelected(match random.below(3) {
            0 => InterfaceSelection::AudioControl,
            1 => InterfaceSelection::SerialControl,
            _ => InterfaceSelection::SerialData,
        }),
        78..=87 => Event::EndpointHalted(endpoint(random)),
        _ => Event::EndpointHaltCleared(endpoint(random)),
    }
}

#[test]
fn streams_run_only_when_configured_at_setting_one_and_reset_or_suspend_idles_them() {
    let mut streaming_steps = 0_u64;
    for seed in 1..=SEQUENCES {
        let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut state = State::INITIAL;
        let mut board = Board::POWER_ON;
        for at in 0..EVENTS_PER_SEQUENCE {
            let event = random_event(&mut random);
            let (after, outputs) = step(state, event);
            board.execute(event, &outputs);
            check(seed, at, state, event, after, board);
            streaming_steps += u64::from(board.playback == AlternateSetting::One);
            state = after;
        }
    }
    assert!(
        streaming_steps > SEQUENCES * EVENTS_PER_SEQUENCE as u64 / 40,
        "the sequences reach playback often enough to test it: {streaming_steps}"
    );
}

fn random_packet(random: &mut XorShift) -> ([u8; 8], Vec<u8>) {
    let wide = |random: &mut XorShift, bound: u64| random.below(bound) as u16;
    match random.below(40) {
        0..=11 => (setup(0x01, 11, wide(random, 3), wide(random, 6), 0), vec![]),
        12..=13 => (
            setup(0x00, 9, [1, 1, 1, 0, 2][random.below(5) as usize], 0, 0),
            vec![],
        ),
        14..=15 => (setup(0x00, 5, wide(random, 130), 0, 0), vec![]),
        16..=19 => {
            let request = if random.below(2) == 0 { 1 } else { 3 };
            let endpoint = [0x83, 0x04, 0x84, 0x01, 0x81, 0x82, 0x00][random.below(7) as usize];
            (setup(0x02, request, 0, endpoint, 0), vec![])
        }
        20..=22 => (setup(0x81, 10, 0, wide(random, 6), 1), vec![]),
        23..=25 => (
            setup(0x21, 0x01, 0x0100, 0x0A00, 4),
            random.next().to_le_bytes()[..4].to_vec(),
        ),
        _ => {
            let mut packet = [0; 8];
            packet.iter_mut().for_each(|byte| *byte = random.byte());
            let length = usize::from(u16::from_le_bytes([packet[6], packet[7]]));
            let data = if packet[0] & 0x80 == 0 && length <= 16 {
                (0..length).map(|_| random.byte()).collect()
            } else {
                vec![]
            };
            (packet, data)
        }
    }
}

fn random_bus_event(random: &mut XorShift) -> Event {
    match random.below(20) {
        0..=4 => Event::Attached,
        5 => Event::Detached,
        6..=8 => Event::BusReset(if random.below(4) == 0 {
            BusSpeed::Full
        } else {
            BusSpeed::High
        }),
        9..=10 => Event::Suspended,
        _ => Event::Resumed,
    }
}

#[test]
fn requests_and_bus_events_never_start_a_stream_outside_a_configured_setting_one() {
    let mut streaming_steps = 0_u64;
    let mut applied_steps = 0_u64;
    for seed in 1..=SEQUENCES {
        let mut random = XorShift(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let mut state = State::INITIAL;
        let mut board = Board::POWER_ON;
        for at in 0..EVENTS_PER_SEQUENCE {
            let event = if random.below(40) == 0 {
                random_bus_event(&mut random)
            } else {
                let (packet, data) = random_packet(&mut random);
                match answer(packet, &data, &context(state)) {
                    Answer::Accept(Effect::Function(event)) => event,
                    Answer::Accept(_) | Answer::Reply(_) | Answer::Stall => continue,
                }
            };
            let (after, outputs) = step(state, event);
            board.execute(event, &outputs);
            check(seed, at, state, event, after, board);
            streaming_steps += u64::from(board.playback == AlternateSetting::One);
            applied_steps += 1;
            state = after;
        }
    }
    assert!(
        streaming_steps > applied_steps / 40,
        "the sequences reach playback often enough to test it: {streaming_steps} of {applied_steps}"
    );
}
