// WHY: These pin the sound2 screen; sound2-ui draws screen2 (tests/lcd_screen2.rs)
#![cfg(not(feature = "screen2"))]

use device::fault::{FaultCode, I2cPeer};
use device::lcd::panel::{InitAction, Opcode};
use device::lcd::scheduler::{
    DirtyRegions, Event, FaultProgress, InitCursor, Output, State, start, transition,
};
use device::lcd::screen::{BACKGROUND, FAULT_BACKGROUND, Region, RegionSet};
use device::phantom::Milliseconds;

const BACKLIGHT_STEP: usize = 27;
const DISPLAY_ON_STEP: usize = 26;
const CLEAR_STEP: usize = 25;
const MEMORY_WRITE_STEP: usize = 24;
const RESET_HOLD_STEP: usize = 3;

fn region_label(region: Region) -> &'static str {
    match region {
        Region::Rate => "rate",
        Region::Stream => "stream",
        Region::Phantom => "phantom",
        Region::Meters => "meters",
        Region::Legend => "legend",
        Region::Dsp => "dsp",
    }
}

fn set_label(regions: RegionSet) -> String {
    let names: Vec<&str> = Region::ALL
        .into_iter()
        .filter(|region| regions.contains(*region))
        .map(region_label)
        .collect();
    if names.is_empty() {
        "{}".into()
    } else {
        format!("{{{}}}", names.join(","))
    }
}

fn cursor_label(cursor: InitCursor) -> String {
    match cursor.remaining_hold() {
        Some(Milliseconds(left)) => format!("step{} hold{left}", cursor.step()),
        None => format!("step{}", cursor.step()),
    }
}

fn progress_label(progress: FaultProgress) -> String {
    match progress {
        FaultProgress::FinishingInit(cursor) => format!("finishing {}", cursor_label(cursor)),
        FaultProgress::AwaitingTransfer => "awaiting-transfer".into(),
        FaultProgress::AwaitingTeToClear => "te-to-clear".into(),
        FaultProgress::Clearing => "clearing".into(),
        FaultProgress::AwaitingTeToWriteCode => "te-to-write".into(),
        FaultProgress::WritingCode => "writing".into(),
        FaultProgress::Shown => "shown".into(),
    }
}

fn state_label(state: State) -> String {
    match state {
        State::Initialising(cursor) => format!("init {}", cursor_label(cursor)),
        State::Idle { next } => format!("idle next={}", region_label(next)),
        State::AwaitingTe { dirty, next } => format!(
            "await {} next={}",
            set_label(dirty.regions()),
            region_label(next)
        ),
        State::Sending { region, dirty } => {
            format!("send {} {}", region_label(region), set_label(dirty))
        }
        State::FaultScreen { code, progress } => {
            format!("fault{} {}", code.number(), progress_label(progress))
        }
    }
}

fn event_label(event: Event) -> String {
    match event {
        Event::StepDone => "done".into(),
        Event::Elapsed(Milliseconds(count)) => format!("elapsed{count}"),
        Event::RegionsDirty(regions) => format!("dirty{}", set_label(regions)),
        Event::TeEdge => "te".into(),
        Event::TransferDone => "transfer-done".into(),
        Event::FaultRaised(code) => format!("fault{}", code.number()),
    }
}

fn output_label(output: Output) -> String {
    match output {
        Output::Init(InitAction::ConfigureSpi) => "configure-spi".into(),
        Output::Init(InitAction::DriveP1_6High) => "p1_6-high".into(),
        Output::Init(InitAction::Send(command)) => format!("cmd{:02X}", command.opcode.byte()),
        Output::Init(InitAction::BacklightOn) => "backlight-on".into(),
        Output::ClearScreen(colour) if colour == BACKGROUND => "clear-background".into(),
        Output::ClearScreen(colour) if colour == FAULT_BACKGROUND => "clear-fault".into(),
        Output::ClearScreen(colour) => format!("clear{:04X}", colour.bits()),
        Output::SendRegion(region) => format!("draw-{}", region_label(region)),
        Output::SendFaultCode(code) => format!("draw-fault{}", code.number()),
    }
}

fn cursor(step: usize) -> InitCursor {
    InitCursor::at(step).expect("a step inside the init list")
}

fn dirty(regions: RegionSet) -> DirtyRegions {
    DirtyRegions::of(regions).expect("a nonempty set")
}

fn partly_held() -> InitCursor {
    match transition(
        State::Initialising(cursor(RESET_HOLD_STEP)),
        Event::Elapsed(Milliseconds(100)),
    )
    .0
    {
        State::Initialising(cursor) => cursor,
        other => panic!("a partial hold stays initialising, got {other:?}"),
    }
}

const SHOWN_CODE: FaultCode = FaultCode::BoardStalled;

fn states() -> Vec<State> {
    let fault = |progress| State::FaultScreen {
        code: SHOWN_CODE,
        progress,
    };
    vec![
        State::Initialising(cursor(0)),
        State::Initialising(cursor(RESET_HOLD_STEP)),
        State::Initialising(partly_held()),
        State::Initialising(cursor(MEMORY_WRITE_STEP)),
        State::Initialising(cursor(CLEAR_STEP)),
        State::Initialising(cursor(DISPLAY_ON_STEP)),
        State::Initialising(cursor(BACKLIGHT_STEP)),
        State::Idle { next: Region::Rate },
        State::Idle { next: Region::Dsp },
        State::AwaitingTe {
            dirty: dirty(RegionSet::of(Region::Meters)),
            next: Region::Rate,
        },
        State::AwaitingTe {
            dirty: dirty(RegionSet::of(Region::Rate).with(Region::Dsp)),
            next: Region::Stream,
        },
        State::Sending {
            region: Region::Meters,
            dirty: RegionSet::EMPTY,
        },
        State::Sending {
            region: Region::Dsp,
            dirty: RegionSet::of(Region::Rate),
        },
        fault(FaultProgress::FinishingInit(cursor(RESET_HOLD_STEP))),
        fault(FaultProgress::FinishingInit(cursor(CLEAR_STEP))),
        fault(FaultProgress::FinishingInit(cursor(DISPLAY_ON_STEP))),
        fault(FaultProgress::FinishingInit(cursor(BACKLIGHT_STEP))),
        fault(FaultProgress::AwaitingTransfer),
        fault(FaultProgress::AwaitingTeToClear),
        fault(FaultProgress::Clearing),
        fault(FaultProgress::AwaitingTeToWriteCode),
        fault(FaultProgress::WritingCode),
        fault(FaultProgress::Shown),
    ]
}

fn events() -> Vec<Event> {
    vec![
        Event::StepDone,
        Event::Elapsed(Milliseconds(100)),
        Event::Elapsed(Milliseconds(150)),
        Event::RegionsDirty(RegionSet::EMPTY),
        Event::RegionsDirty(RegionSet::of(Region::Phantom)),
        Event::TeEdge,
        Event::TransferDone,
        Event::FaultRaised(FaultCode::Unanswered(I2cPeer::Adc)),
    ]
}

fn state_variant(state: State) -> &'static str {
    match state {
        State::Initialising(_) => "Initialising",
        State::Idle { .. } => "Idle",
        State::AwaitingTe { .. } => "AwaitingTe",
        State::Sending { .. } => "Sending",
        State::FaultScreen { progress, .. } => match progress {
            FaultProgress::FinishingInit(_) => "FaultScreen FinishingInit",
            FaultProgress::AwaitingTransfer => "FaultScreen AwaitingTransfer",
            FaultProgress::AwaitingTeToClear => "FaultScreen AwaitingTeToClear",
            FaultProgress::Clearing => "FaultScreen Clearing",
            FaultProgress::AwaitingTeToWriteCode => "FaultScreen AwaitingTeToWriteCode",
            FaultProgress::WritingCode => "FaultScreen WritingCode",
            FaultProgress::Shown => "FaultScreen Shown",
        },
    }
}

fn event_variant(event: Event) -> &'static str {
    match event {
        Event::StepDone => "StepDone",
        Event::Elapsed(_) => "Elapsed",
        Event::RegionsDirty(_) => "RegionsDirty",
        Event::TeEdge => "TeEdge",
        Event::TransferDone => "TransferDone",
        Event::FaultRaised(_) => "FaultRaised",
    }
}

#[test]
fn the_table_reaches_every_state_variant_and_every_event_variant() {
    let mut states: Vec<&str> = states().into_iter().map(state_variant).collect();
    states.dedup();
    assert_eq!(states.len(), 11, "{states:?}");
    let mut events: Vec<&str> = events().into_iter().map(event_variant).collect();
    events.dedup();
    assert_eq!(events.len(), 6, "{events:?}");
}

#[test]
fn the_scheduler_starts_by_configuring_the_spi_bus() {
    let (state, outputs) = start();
    assert_eq!(state, State::Initialising(cursor(0)));
    assert_eq!(
        outputs.as_slice(),
        &[Output::Init(InitAction::ConfigureSpi)]
    );
}

#[test]
fn display_on_follows_the_clear_and_the_backlight_turns_on_last() {
    let (state, outputs) = transition(State::Initialising(cursor(CLEAR_STEP)), Event::TransferDone);
    assert!(matches!(
        outputs.as_slice(),
        [Output::Init(InitAction::Send(command))] if command.opcode == Opcode::DisplayOn
    ));
    let (state, outputs) = transition(state, Event::StepDone);
    assert_eq!(state, State::Initialising(cursor(BACKLIGHT_STEP)));
    assert_eq!(outputs.as_slice(), &[Output::Init(InitAction::BacklightOn)]);
    let (state, outputs) = transition(state, Event::StepDone);
    assert!(matches!(state, State::AwaitingTe { .. }), "{state:?}");
    assert!(outputs.as_slice().is_empty());
}

const TABLE: &str = include_str!("lcd_scheduler_table.txt");

#[test]
fn every_scheduler_state_and_event_pair_has_its_pinned_result() {
    let mut actual = String::new();
    for state in states() {
        for event in events() {
            let (next, outputs) = transition(state, event);
            let produced: Vec<String> = outputs
                .as_slice()
                .iter()
                .map(|output| output_label(*output))
                .collect();
            actual.push_str(&format!(
                "{} | {} => {} | {}\n",
                state_label(state),
                event_label(event),
                state_label(next),
                produced.join(" ")
            ));
        }
    }
    let normalise = |text: &str| -> Vec<String> {
        text.lines()
            .map(|line| line.trim_end().to_string())
            .collect()
    };
    if std::env::var_os("LCD_TABLE_PRINT").is_some() {
        print!("{actual}");
    }
    assert_eq!(normalise(&actual), normalise(TABLE), "actual:\n{actual}");
}
