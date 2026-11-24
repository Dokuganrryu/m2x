mod usb_host_model;

use device::usb::BusSpeed;
use device::usb::endpoint::{
    DmaPositions, Event, FrameStart, InEntry, ListEntry, OutEntry, StreamCompletions,
};
use device::usb::function::{self, Bus, Configuration};
use stream::{AlternateSetting, Frames, TxDmaPosition};
use usb_host_model::{Bench, with_bench};

const SEQUENCES: u64 = 2_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

const SETUPS: [[u8; 8]; 12] = [
    [0x00, 0x05, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x01, 0x0B, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00],
    [0x01, 0x0B, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
    [0x01, 0x0B, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00],
    [0x01, 0x0B, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00],
    [0x02, 0x03, 0x00, 0x00, 0x84, 0x00, 0x00, 0x00],
    [0x02, 0x01, 0x00, 0x00, 0x84, 0x00, 0x00, 0x00],
    [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00],
    [0x21, 0x01, 0x00, 0x01, 0x00, 0x0A, 0x04, 0x00],
    [0x40, b'q', 0x00, 0x00, 0x07, 0x00, 0x00, 0x00],
];

fn sof() -> Event {
    Event::StartOfFrame(FrameStart {
        positions: DmaPositions {
            playback: TxDmaPosition {
                frames_queued: Frames::new(6),
            },
            capture: Frames::new(0),
        },
        completed: StreamCompletions::NONE,
    })
}

fn random_step(bench: &mut Bench<'_>, random: &mut XorShift) {
    match random.below(32) {
        0..=7 => {
            bench.deliver(Event::Setup(
                SETUPS[random.below(SETUPS.len() as u64) as usize],
            ));
            if random.below(4) != 0 {
                bench.host_completes(ListEntry::In(InEntry::Control), 0);
            }
        }
        8..=19 => {
            let entry = ListEntry::ALL[random.below(ListEntry::ALL.len() as u64) as usize];
            bench.host_completes(entry, random.next() as u8);
        }
        20..=27 => bench.deliver(sof()),
        28 => {
            if random.below(2) == 0 {
                enumerate_and_stream(bench);
            } else {
                bench.deliver(Event::BusReset(BusSpeed::Full));
            }
        }
        29 => bench.deliver(Event::Suspended),
        30 => bench.deliver(Event::Resumed),
        _ => bench.deliver(if random.below(2) == 0 {
            Event::Attached
        } else {
            Event::Detached
        }),
    }
}

fn enumerate_and_stream(bench: &mut Bench<'_>) {
    bench.attach_and_reset(BusSpeed::High);
    for request in [SETUPS[0], SETUPS[1], SETUPS[3], SETUPS[5]] {
        let _ = bench.control_write(request, &[]);
    }
}

fn streams_on(state: function::State) -> (bool, bool) {
    match state {
        function::State::Awake(Bus::Configured(Configuration::HighSpeed(audio))) => (
            audio.playback() == AlternateSetting::One,
            audio.capture() == AlternateSetting::One,
        ),
        _ => (false, false),
    }
}

fn configured(state: function::State) -> bool {
    matches!(
        state,
        function::State::Awake(Bus::Configured(_)) | function::State::Suspended(Bus::Configured(_))
    )
}

fn walk(seed: u64, mut check: impl FnMut(&Bench<'_>)) {
    with_bench(|bench| {
        let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        enumerate_and_stream(bench);
        for _ in 0..EVENTS_PER_SEQUENCE {
            random_step(bench, &mut random);
            check(bench);
        }
    });
}

#[test]
fn an_isochronous_buffer_is_armed_only_while_its_stream_runs() {
    let mut playing_steps = 0;
    for seed in 0..SEQUENCES {
        walk(seed, |bench| {
            let (playing, capturing) = streams_on(bench.service.function());
            playing_steps += u64::from(playing);
            for entry in ListEntry::ALL {
                let allowed = match entry {
                    ListEntry::Out(OutEntry::Playback(_)) | ListEntry::In(InEntry::Feedback) => {
                        playing
                    }
                    ListEntry::In(InEntry::Capture(_)) => capturing,
                    ListEntry::Out(OutEntry::Serial) | ListEntry::In(InEntry::Serial) => {
                        configured(bench.service.function())
                    }
                    ListEntry::Out(OutEntry::Control)
                    | ListEntry::In(InEntry::Control)
                    | ListEntry::Setup => true,
                };
                assert!(
                    allowed || !bench.is_armed(entry),
                    "seed {seed}: {entry:?} armed in {:?}",
                    bench.service.function()
                );
            }
        });
    }
    assert!(
        playing_steps > SEQUENCES * EVENTS_PER_SEQUENCE as u64 / 10,
        "playback ran in {playing_steps} steps"
    );
}

#[test]
fn a_bus_reset_or_a_detach_leaves_no_buffer_with_the_controller() {
    for seed in 0..SEQUENCES / 4 {
        with_bench(|bench| {
            let mut random = XorShift(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
            bench.attach_and_reset(BusSpeed::High);
            for _ in 0..EVENTS_PER_SEQUENCE {
                random_step(bench, &mut random);
                if random.below(50) == 0 {
                    bench.deliver(Event::BusReset(BusSpeed::High));
                    assert!(bench.armed_entries().is_empty(), "seed {seed}");
                    assert!(!bench.control_stalled, "seed {seed}");
                }
            }
        });
    }
}
