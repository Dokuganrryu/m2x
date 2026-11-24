mod usb_control_support;
mod usb_support;

use device::usb::answer::{ControlContext, Descriptors, Effect};
use device::usb::control::{CONTROL_PACKET_BYTES, Event, Output, State, transition};
use device::usb::function;
use device::usb::{
    BENCH_DEVICE_DESCRIPTOR, BusSpeed, DescriptorText, DeviceStrings, FACTORY_SERIAL_TEMPLATE,
};
use usb_control_support::*;
use usb_support::{XorShift, context};

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;

const TEMPLATES: [[u8; 8]; 15] = [
    GET_DEVICE_DESCRIPTOR_64,
    GET_DEVICE_DESCRIPTOR_18,
    GET_DEVICE_DESCRIPTOR_NO_DATA,
    GET_CONFIGURATION_DESCRIPTOR_255,
    GET_CONFIGURATION_DESCRIPTOR_ALL,
    GET_CONFIGURATION_DESCRIPTOR_9,
    GET_PRODUCT_STRING_255,
    GET_PRODUCT_STRING_64,
    VENDOR_OUT_Q,
    SET_ADDRESS_9,
    SET_CONFIGURATION_1,
    SET_PLAYBACK_ALTERNATE_1,
    SET_CUR_SAMPLING_FREQUENCY,
    GET_CUR_SAMPLING_FREQUENCY,
    SET_LINE_CODING,
];

static STRINGS: std::sync::LazyLock<DeviceStrings> = std::sync::LazyLock::new(|| DeviceStrings {
    manufacturer: DescriptorText::new("m2x").unwrap(),
    product: DescriptorText::new("abcdefghijklmnopqrstuvwxyz01234").unwrap(),
});

fn context_for(function: function::State) -> ControlContext<'static> {
    ControlContext {
        descriptors: Descriptors {
            device: &BENCH_DEVICE_DESCRIPTOR,
            strings: &STRINGS,
            serial: FACTORY_SERIAL_TEMPLATE,
        },
        ..context(function)
    }
}

enum Generated {
    Setup([u8; 8]),
    InSent,
    Out(Vec<u8>),
    BusReset,
}

fn generate(random: &mut XorShift) -> Generated {
    match random.below(10) {
        0..=2 => {
            let mut packet = TEMPLATES[random.below(TEMPLATES.len() as u64) as usize];
            match random.below(4) {
                0 => {
                    let at = random.below(8) as usize;
                    packet[at] = random.byte();
                }
                1 => packet = [(); 8].map(|_| random.byte()),
                _ => {}
            }
            Generated::Setup(packet)
        }
        3..=5 => Generated::InSent,
        6..=8 => Generated::Out(match random.below(4) {
            0 => LINE_CODING_115200_8N1.to_vec(),
            1 => NINETY_SIX_KILOHERTZ.to_vec(),
            2 => vec![],
            _ => (0..random.below(71)).map(|_| random.byte()).collect(),
        }),
        _ => Generated::BusReset,
    }
}

const fn requested_length(setup: [u8; 8]) -> usize {
    u16::from_le_bytes([setup[6], setup[7]]) as usize
}

struct Transfer {
    setup: [u8; 8],
    replied_bytes: usize,
    data_received: usize,
    stalled: bool,
    effects: usize,
}

impl Transfer {
    fn after(setup: [u8; 8]) -> Self {
        Self {
            setup,
            replied_bytes: 0,
            data_received: 0,
            stalled: false,
            effects: 0,
        }
    }

    fn reads(&self) -> bool {
        self.setup[0] & 0x80 != 0
    }
}

fn walk(
    seed: u64,
    mut check: impl FnMut(
        &State<'static>,
        &Event<'_>,
        &[Output],
        &ControlContext<'static>,
        &mut Transfer,
    ),
) {
    let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut function = usb_support::enumerated_at(BusSpeed::High);
    let mut state = State::INITIAL;
    let mut transfer = Transfer::after([0; 8]);
    for _ in 0..EVENTS_PER_SEQUENCE {
        let generated = generate(&mut random);
        let context = context_for(function);
        let data;
        let event = match &generated {
            Generated::Setup(packet) => Event::Setup(*packet),
            Generated::InSent => Event::InSent,
            Generated::Out(bytes) => {
                data = bytes.clone();
                Event::OutReceived(&data)
            }
            Generated::BusReset => Event::BusReset,
        };
        if let Event::Setup(packet) = event {
            transfer = Transfer::after(packet);
        }
        let (next, outputs) = transition(state, event, &context);
        check(&state, &event, outputs.as_slice(), &context, &mut transfer);
        for output in outputs.as_slice() {
            if let Output::Effect(Effect::Function(event)) = output {
                function = function::transition(function, *event).0;
            }
        }
        if let Event::BusReset = event {
            function = function::transition(function, function::Event::BusReset(BusSpeed::High)).0;
            if random.below(2) == 0 {
                function = usb_support::enumerated_at(BusSpeed::High);
            }
        }
        state = next;
    }
}

#[test]
fn a_reply_never_exceeds_w_length_and_no_packet_exceeds_the_control_packet_size() {
    for seed in 0..SEQUENCES {
        walk(seed, |_, _, outputs, _, transfer| {
            for output in outputs {
                if let Output::Send(packet) = output {
                    assert!(packet.bytes().len() <= CONTROL_PACKET_BYTES, "seed {seed}");
                    if transfer.reads() {
                        transfer.replied_bytes += packet.bytes().len();
                        assert!(
                            transfer.replied_bytes <= requested_length(transfer.setup),
                            "seed {seed}: {:02X?} got {} bytes",
                            transfer.setup,
                            transfer.replied_bytes
                        );
                    } else {
                        assert!(
                            packet.bytes().is_empty(),
                            "seed {seed}: a write gets only a status packet"
                        );
                    }
                }
            }
        });
    }
}

#[test]
fn a_stall_is_cleared_only_by_the_next_setup() {
    for seed in 0..SEQUENCES {
        walk(seed, |_, event, outputs, _, transfer| {
            let setup = matches!(event, Event::Setup(_));
            let reset = matches!(event, Event::BusReset);
            for (at, output) in outputs.iter().enumerate() {
                if *output == Output::ReleaseStall {
                    assert!(setup && at == 0, "seed {seed}: release outside a setup");
                }
            }
            if transfer.stalled && !setup && !reset {
                assert!(outputs.is_empty(), "seed {seed}: {outputs:?} after a stall");
            }
            if outputs.contains(&Output::Stall) {
                transfer.stalled = true;
            }
        });
    }
}

#[test]
fn no_write_effect_leaves_before_its_data_stage_has_fully_arrived_and_its_status_was_sent() {
    let mut effects = 0;
    for seed in 0..SEQUENCES {
        walk(seed, |state, event, outputs, _, transfer| {
            if let (State::ReceivingData(_), Event::OutReceived(bytes)) = (state, event) {
                transfer.data_received += bytes.len();
            }
            for output in outputs {
                if let Output::Effect(_) = output {
                    effects += 1;
                    transfer.effects += 1;
                    assert_eq!(
                        *event,
                        Event::InSent,
                        "seed {seed}: effect off the status stage"
                    );
                    assert!(matches!(state, State::ConfirmingWrite(_)), "seed {seed}");
                    assert!(!transfer.reads(), "seed {seed}: a read has no effect");
                    assert_eq!(
                        transfer.data_received,
                        requested_length(transfer.setup),
                        "seed {seed}: {:02X?}",
                        transfer.setup
                    );
                    assert_eq!(transfer.effects, 1, "seed {seed}: one effect per setup");
                }
            }
        });
    }
    assert!(effects > 10_000, "the walk reached {effects} effects");
}

#[test]
fn a_setup_in_any_state_gives_the_same_result_as_a_setup_on_a_clean_machine() {
    let mut setups_off_idle = 0;
    for seed in 0..SEQUENCES {
        walk(seed, |state, event, outputs, context, _| {
            if let Event::Setup(packet) = event {
                let (clean_state, clean_outputs) =
                    transition(State::INITIAL, Event::Setup(*packet), context);
                let (next, _) = transition(*state, *event, context);
                assert_eq!(next, clean_state, "seed {seed}");
                assert_eq!(outputs, clean_outputs.as_slice(), "seed {seed}");
                if *state != State::Idle {
                    setups_off_idle += 1;
                }
            }
        });
    }
    assert!(
        setups_off_idle > 100_000,
        "{setups_off_idle} setups arrived mid-transfer"
    );
}
