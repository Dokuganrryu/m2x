use device::fault::FaultCode;
use device::lcd::panel::{InitAction, Opcode};
use device::lcd::scheduler::{
    Event, FaultProgress, MOST_READY_TE_EDGES_BEFORE_DRAW, Output, State, start, transition,
};
use device::lcd::screen::{BACKGROUND, FAULT_BACKGROUND, Region, RegionSet};
use device::phantom::Milliseconds;

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;
const DRAIN_EVENT_LIMIT: usize = 400;

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

    fn regions(&mut self) -> RegionSet {
        Region::ALL
            .into_iter()
            .filter(|_| self.below(4) == 0)
            .fold(RegionSet::EMPTY, RegionSet::with)
    }
}

#[derive(Default)]
struct Board {
    step_pending: bool,
    transfer_in_flight: bool,
    init_done: bool,
    display_on_sent: bool,
    #[cfg(feature = "screen2")]
    batched_since_init: bool,
    backlight_on_sent: bool,
    fault: Option<FaultCode>,
    fault_clear_sent: bool,
    fault_code_sent: bool,
    waiting: [Option<usize>; Region::ALL.len()],
}

fn slot(region: Region) -> usize {
    Region::ALL
        .iter()
        .position(|candidate| *candidate == region)
        .expect("a listed region")
}

impl Board {
    fn ready_for_te(&self) -> bool {
        self.init_done && !self.transfer_in_flight && self.fault.is_none()
    }

    fn pick(&self, rng: &mut XorShift) -> Event {
        loop {
            let event = match rng.below(100) {
                0..=29 => Event::TeEdge,
                30..=44 => Event::Elapsed(Milliseconds(1 + rng.below(200) as u32)),
                45..=69 => Event::RegionsDirty(rng.regions()),
                70..=84 if self.step_pending => Event::StepDone,
                85..=98 if self.transfer_in_flight => Event::TransferDone,
                99 if rng.below(40) == 0 => {
                    Event::FaultRaised(FaultCode::ALL[rng.below(8) as usize])
                }
                _ => continue,
            };
            return event;
        }
    }

    fn faithful(&self) -> Event {
        if self.step_pending {
            Event::StepDone
        } else if self.transfer_in_flight {
            Event::TransferDone
        } else if !self.init_done {
            Event::Elapsed(Milliseconds(150))
        } else {
            Event::TeEdge
        }
    }

    fn observe(&mut self, seed: u64, state_before: State, event: Event, outputs: &[Output]) {
        let was_ready = self.ready_for_te();
        match event {
            Event::StepDone => {
                assert!(
                    self.step_pending,
                    "seed {seed}: harness sent a stray StepDone"
                );
                self.step_pending = false;
                if self.backlight_on_sent {
                    self.init_done = true;
                }
            }
            Event::TransferDone => self.transfer_in_flight = false,
            Event::RegionsDirty(regions) => {
                for region in Region::ALL {
                    if regions.contains(region) && self.waiting[slot(region)].is_none() {
                        self.waiting[slot(region)] = Some(0);
                    }
                }
            }
            Event::FaultRaised(code) => {
                if self.fault.is_none() {
                    self.fault = Some(code);
                }
            }
            Event::TeEdge => {
                if was_ready {
                    for edges in self.waiting.iter_mut().flatten() {
                        *edges += 1;
                    }
                }
            }
            Event::Elapsed(_) => {}
        }
        for output in outputs {
            match *output {
                Output::Init(action) => {
                    assert!(!self.step_pending, "seed {seed}: two init actions at once");
                    assert!(!self.init_done, "seed {seed}: init action after init");
                    self.step_pending = true;
                    if matches!(action, InitAction::Send(command) if command.opcode == Opcode::DisplayOn)
                    {
                        self.display_on_sent = true;
                    }
                    if action == InitAction::BacklightOn {
                        assert!(
                            self.display_on_sent,
                            "seed {seed}: backlight on before display on"
                        );
                        self.backlight_on_sent = true;
                    }
                }
                Output::ClearScreen(colour) => {
                    self.start_transfer(seed);
                    if colour == FAULT_BACKGROUND {
                        assert!(
                            self.fault.is_some(),
                            "seed {seed}: fault clear without fault"
                        );
                        assert!(self.init_done, "seed {seed}: fault clear before init ended");
                        assert!(!self.fault_clear_sent, "seed {seed}: second fault clear");
                        assert_eq!(event, Event::TeEdge, "seed {seed}: clear off a TE edge");
                        self.fault_clear_sent = true;
                    } else {
                        assert_eq!(colour, BACKGROUND, "seed {seed}");
                        assert!(!self.init_done, "seed {seed}: background clear after init");
                    }
                }
                Output::SendRegion(region) => {
                    self.start_transfer(seed);
                    assert!(
                        self.init_done,
                        "seed {seed}: pixels before the init list ended"
                    );
                    assert!(
                        self.fault.is_none(),
                        "seed {seed}: drew {region:?} after a fault"
                    );
                    assert_eq!(event, Event::TeEdge, "seed {seed}: transfer off a TE edge");
                    let edges = self.waiting[slot(region)].take();
                    assert!(
                        edges.is_some_and(|edges| edges <= MOST_READY_TE_EDGES_BEFORE_DRAW),
                        "seed {seed}: {region:?} drawn after {edges:?} ready TE edges"
                    );
                }
                #[cfg(feature = "screen2")]
                Output::SendBatch(regions) => {
                    self.start_transfer(seed);
                    assert!(
                        self.init_done,
                        "seed {seed}: pixels before the init list ended"
                    );
                    assert!(
                        self.fault.is_none(),
                        "seed {seed}: drew {regions:?} after a fault"
                    );
                    assert_eq!(event, Event::TeEdge, "seed {seed}: transfer off a TE edge");
                    let first_after_init = !self.batched_since_init;
                    self.batched_since_init = true;
                    for region in Region::ALL.into_iter().filter(|r| regions.contains(*r)) {
                        let edges = self.waiting[slot(region)].take();
                        // WHY: Scheduler.rs initialising, the init list ends with every region dirty, which the random events never announce
                        assert!(
                            edges.is_some_and(|edges| edges <= 1)
                                || (first_after_init && edges.is_none()),
                            "seed {seed}: {region:?} batched after {edges:?} ready TE edges"
                        );
                    }
                    assert!(
                        self.waiting.iter().all(Option::is_none),
                        "seed {seed}: a dirty region was left out of the batch"
                    );
                }
                Output::SendFaultCode(code) => {
                    self.start_transfer(seed);
                    assert_eq!(
                        Some(code),
                        self.fault,
                        "seed {seed}: wrong fault code drawn"
                    );
                    assert!(
                        self.fault_clear_sent,
                        "seed {seed}: code before the fault clear"
                    );
                    assert!(!self.fault_code_sent, "seed {seed}: second fault code");
                    assert_eq!(event, Event::TeEdge, "seed {seed}: code off a TE edge");
                    self.fault_code_sent = true;
                }
            }
        }
        if self.fault.is_none() {
            for (region, edges) in Region::ALL.into_iter().zip(self.waiting) {
                assert!(
                    edges.is_none_or(|edges| edges < MOST_READY_TE_EDGES_BEFORE_DRAW),
                    "seed {seed}: {region:?} still waiting after {edges:?} ready TE edges in {state_before:?}"
                );
            }
        }
    }

    fn start_transfer(&mut self, seed: u64) {
        assert!(
            !self.transfer_in_flight,
            "seed {seed}: a transfer started while another was in flight"
        );
        self.transfer_in_flight = true;
    }

    fn settled(&self, state: State) -> bool {
        match self.fault {
            Some(code) => {
                state
                    == State::FaultScreen {
                        code,
                        progress: FaultProgress::Shown,
                    }
            }
            None => {
                self.init_done
                    && self.waiting.iter().all(Option::is_none)
                    && matches!(state, State::Idle { .. })
            }
        }
    }
}

fn run(seed: u64, mut rng: XorShift) {
    let mut board = Board::default();
    let (mut state, first) = start();
    board.observe(
        seed,
        state,
        Event::Elapsed(Milliseconds(0)),
        first.as_slice(),
    );
    for _ in 0..EVENTS_PER_SEQUENCE {
        let event = board.pick(&mut rng);
        let (next, outputs) = transition(state, event);
        board.observe(seed, state, event, outputs.as_slice());
        state = next;
    }
    for _ in 0..DRAIN_EVENT_LIMIT {
        if board.settled(state) {
            return;
        }
        let event = board.faithful();
        let (next, outputs) = transition(state, event);
        board.observe(seed, state, event, outputs.as_slice());
        state = next;
    }
    panic!("seed {seed}: never settled, ended in {state:?}");
}

#[test]
fn every_scheduler_invariant_holds_over_random_event_sequences() {
    for seed in 1..=SEQUENCES {
        run(seed, XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1));
    }
}

#[cfg(not(feature = "screen2"))]
#[test]
fn a_dirty_region_is_drawn_within_six_ready_te_edges() {
    assert_eq!(MOST_READY_TE_EDGES_BEFORE_DRAW, 6);
}

#[test]
fn a_fault_raised_mid_init_finishes_the_init_list_then_shows_the_code() {
    let mut board = Board::default();
    let (mut state, first) = start();
    board.observe(0, state, Event::Elapsed(Milliseconds(0)), first.as_slice());
    for event in [Event::StepDone, Event::FaultRaised(FaultCode::Panic)] {
        let (next, outputs) = transition(state, event);
        board.observe(0, state, event, outputs.as_slice());
        state = next;
    }
    for _ in 0..DRAIN_EVENT_LIMIT {
        if board.settled(state) {
            break;
        }
        let event = board.faithful();
        let (next, outputs) = transition(state, event);
        board.observe(0, state, event, outputs.as_slice());
        state = next;
    }
    assert!(board.display_on_sent && board.backlight_on_sent && board.fault_code_sent);
    assert_eq!(
        state,
        State::FaultScreen {
            code: FaultCode::Panic,
            progress: FaultProgress::Shown
        }
    );
}
