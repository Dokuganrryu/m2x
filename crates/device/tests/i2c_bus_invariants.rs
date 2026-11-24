use device::adc::{AdcAddress, AdcWrite};
use device::command::Command;
use device::dac::{Attenuation, DacWrite, MasterTrim, StatusRegister, VolumeRegister};
use device::i2c::{Answer, BusAction, Event, MasterStatus, Output, State, Transfer, transition};

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 400;

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

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const COMMANDS: [Command; 6] = [
    Command::WriteDac(DacWrite::Volume(VolumeRegister::MainA, Attenuation::FULL)),
    Command::WriteDac(DacWrite::MasterTrim(MasterTrim::MINUS_ONE_DB)),
    Command::ReadDac(StatusRegister::Register113),
    Command::ProbeAdc(AdcAddress::At13),
    Command::WriteAdc(AdcWrite::At13Register2),
    Command::WriteExpanderDirection,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wire {
    Idle,
    AddressSent { reading: bool, acked: bool },
    ByteSent { acked: bool },
    Stopped,
}

struct Hardware {
    wire: Wire,
    pending_byte: Option<u8>,
    in_transaction: bool,
    transfer_clean: bool,
    read_byte: Option<u8>,
    nack_percent: u64,
}

impl Hardware {
    fn new(nack_percent: u64) -> Self {
        Self {
            wire: Wire::Idle,
            pending_byte: None,
            in_transaction: false,
            transfer_clean: true,
            read_byte: None,
            nack_percent,
        }
    }

    fn act(&mut self, action: BusAction, rng: &mut XorShift) {
        match action {
            BusAction::Transmit(byte) => self.pending_byte = Some(byte),
            BusAction::Start => {
                let byte = self
                    .pending_byte
                    .take()
                    .expect("a start follows its address byte");
                let acked = !rng.chance(self.nack_percent);
                if !acked {
                    self.transfer_clean = false;
                }
                let restart = self.in_transaction;
                assert!(
                    !restart || byte & 1 == 1,
                    "a repeated start without a stop is only the turn to reading"
                );
                self.in_transaction = true;
                self.wire = Wire::AddressSent {
                    reading: byte & 1 == 1,
                    acked,
                };
            }
            BusAction::Continue => {
                assert!(self.in_transaction, "continue only inside a transaction");
                self.pending_byte
                    .take()
                    .expect("a continue follows its data byte");
                let acked = !rng.chance(self.nack_percent);
                if !acked {
                    self.transfer_clean = false;
                }
                self.wire = Wire::ByteSent { acked };
            }
            BusAction::Stop => {
                assert!(self.in_transaction, "stop only inside a transaction");
                self.in_transaction = false;
                self.wire = Wire::Stopped;
            }
            BusAction::ResetMaster => {
                self.in_transaction = false;
                self.pending_byte = None;
                self.wire = Wire::Idle;
            }
        }
    }

    fn status(&mut self, rng: &mut XorShift) -> MasterStatus {
        if rng.chance(20) {
            return MasterStatus::InProgress;
        }
        if rng.chance(1) {
            self.transfer_clean = false;
            return MasterStatus::Disrupted;
        }
        match self.wire {
            Wire::Idle | Wire::Stopped => MasterStatus::Idle,
            Wire::AddressSent { acked: false, .. } => MasterStatus::NackAddress,
            Wire::AddressSent {
                reading: true,
                acked: true,
            } => {
                let byte = rng.below(256) as u8;
                self.read_byte = Some(byte);
                MasterStatus::ReceiveReady(byte)
            }
            Wire::AddressSent {
                reading: false,
                acked: true,
            }
            | Wire::ByteSent { acked: true } => MasterStatus::TransmitReady,
            Wire::ByteSent { acked: false } => MasterStatus::NackData,
        }
    }
}

#[test]
fn every_bus_invariant_holds_over_random_transfers_and_answers() {
    let mut answered_acked = 0;
    let mut answered_nacked = 0;
    let mut answered_read = 0;
    let mut interrupted = 0;
    for seed in 1..=SEQUENCES {
        let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut hardware = Hardware::new([0, 3, 30][(seed % 3) as usize]);
        let mut state = State::Idle;
        let mut in_flight: Option<Command> = None;
        for _ in 0..EVENTS_PER_SEQUENCE {
            let begin_now = match in_flight {
                None => rng.chance(30),
                Some(_) => rng.chance(1),
            };
            let event = if begin_now {
                let command = COMMANDS[rng.below(COMMANDS.len() as u64) as usize];
                if in_flight.is_some() {
                    interrupted += 1;
                }
                in_flight = Some(command);
                hardware.transfer_clean = true;
                hardware.read_byte = None;
                Event::Begin(Transfer::of(command).expect("bus command"))
            } else {
                Event::Polled(hardware.status(&mut rng))
            };
            let (next, outputs) = transition(state, event);
            state = next;
            let mut answers = 0;
            for output in outputs.as_slice() {
                match *output {
                    Output::Act(action) => hardware.act(action, &mut rng),
                    Output::Answered(answer) => {
                        answers += 1;
                        let command = in_flight.take().expect("an answer belongs to a transfer");
                        assert_eq!(state, State::Idle, "an answer leaves the bus idle");
                        match answer {
                            Answer::Acked => {
                                assert!(
                                    hardware.transfer_clean,
                                    "acked only when every byte was acked"
                                );
                                assert!(
                                    !matches!(command, Command::ReadDac(_)),
                                    "a read answers with its byte"
                                );
                                answered_acked += 1;
                            }
                            Answer::Read(byte) => {
                                assert!(matches!(command, Command::ReadDac(_)));
                                assert!(hardware.transfer_clean);
                                assert_eq!(
                                    Some(byte),
                                    hardware.read_byte,
                                    "the byte the part sent"
                                );
                                answered_read += 1;
                            }
                            Answer::Nacked => {
                                assert!(
                                    !hardware.transfer_clean,
                                    "nacked only after a nack or a disruption"
                                );
                                answered_nacked += 1;
                            }
                        }
                    }
                }
            }
            assert!(answers <= 1);
            assert_eq!(
                in_flight.is_none(),
                state == State::Idle,
                "the bus is busy exactly while a transfer waits for its answer"
            );
        }
    }
    assert!(answered_acked > 10_000);
    assert!(answered_nacked > 10_000);
    assert!(answered_read > 5_000);
    assert!(interrupted > 1_000);
}
