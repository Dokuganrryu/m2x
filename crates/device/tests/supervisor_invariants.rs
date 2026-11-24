use device::breadcrumb::{At, Weight};
use device::command::Command;
use device::fault::FaultCode;
use device::output_pin::OutputPin;
use device::phantom::PinLevel;
use device::supervisor::{Event, Output, State, Usb, transition};

const SEQUENCES: u64 = 10_000;
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

    fn event(&mut self) -> Event {
        match self.below(100) {
            0..=19 => Event::TickAdvanced,
            20..=39 => Event::ControlPassDone,
            40..=54 => Event::UsbServiced,
            55..=59 => Event::UsbBus(Usb::Serving),
            60..=64 => Event::UsbBus(Usb::Silent),
            65..=67 => Event::TransportStarted,
            68..=72 => Event::FaultRaised(FaultCode::ALL[self.below(8) as usize], At::Here),
            _ => Event::FeedWindowEnded,
        }
    }
}

#[derive(Default)]
struct Model {
    transport_running: bool,
    usb_attached: bool,
    tick: bool,
    pass: bool,
    usb: bool,
    fault: Option<FaultCode>,
}

impl Model {
    fn feed_due(&self) -> bool {
        self.pass && (self.tick || !self.transport_running) && (self.usb || !self.usb_attached)
    }
}

#[test]
fn every_supervisor_invariant_holds_over_random_event_sequences() {
    for seed in 1..=SEQUENCES {
        let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut model = Model::default();
        let mut state = State::START;
        for _ in 0..EVENTS_PER_SEQUENCE {
            let event = rng.event();
            let (next, outputs) = transition(state, event);
            let outputs = outputs.as_slice();
            let fed = outputs.contains(&Output::Command(Command::FeedWatchdog));
            match event {
                Event::TickAdvanced => model.tick = true,
                Event::ControlPassDone => model.pass = true,
                Event::UsbServiced => model.usb = true,
                Event::UsbBus(Usb::Serving) => model.usb_attached = true,
                Event::UsbBus(Usb::Silent) => model.usb_attached = false,
                Event::TransportStarted => model.transport_running = true,
                Event::FaultRaised(code, at) => match model.fault {
                    Some(_) => assert!(outputs.is_empty(), "seed {seed}: second fault emitted"),
                    None => {
                        model.fault = Some(code);
                        model.transport_running = false;
                        let expected = [
                            Output::Command(Command::LatchPlaybackSilence),
                            Output::Command(Command::DrivePin(
                                OutputPin::TransportActive,
                                PinLevel::Low,
                            )),
                            Output::Command(Command::StopTickTimer),
                            Output::Command(Command::RecordFault(code, at, Weight::Raised)),
                            Output::FaultRaised(code),
                        ];
                        assert_eq!(outputs, expected, "seed {seed}");
                    }
                },
                Event::FeedWindowEnded => {
                    assert_eq!(fed, model.feed_due(), "seed {seed}: feed decision");
                    if fed {
                        model.tick = false;
                        model.pass = false;
                        model.usb = false;
                    }
                }
            }
            if !matches!(event, Event::FeedWindowEnded) {
                assert!(!fed, "seed {seed}: fed outside a window end");
            }
            if !matches!(event, Event::FaultRaised(..) | Event::FeedWindowEnded) {
                assert!(
                    outputs
                        .iter()
                        .all(|o| *o == Output::Command(Command::FeedWatchdog)),
                    "seed {seed}: unexpected {outputs:?}"
                );
            }
            if let Some(code) = model.fault {
                assert!(
                    matches!(next, State::Faulted { code: held, .. } if held == code),
                    "seed {seed}: faulted state left or code changed"
                );
            }
            state = next;
        }
    }
}
