use device::boot::BOOT_RATE;
use device::breadcrumb::Breadcrumb;
use device::command::Command;
use device::control_pass::{Monitoring, PanelReadings};
use device::expander::{ExpanderInputs, ExpanderOutputs};
use device::fault::FaultCode;
use device::i2c::Answer;
use device::input::Input;
use device::knob::KnobReadings;
use device::knob_scan;
use device::lcd::meter::METER_CHANNELS;
use device::lcd::panel::InitAction;
use device::lcd::scheduler;
use device::lcd::screen::DspBadge;
use device::lcd::screen::{ClockLock, MeterFrame, Screen, StreamActivity};
#[cfg(feature = "sound2-ui")]
use device::lcd::screen::{Glass, PillFade};
use device::panel_bus::{self, PanelTransfer};
use device::phantom::{Jack, Milliseconds, PhantomGrant, PhantomPower};
use device::pin::{Pin, jack_sense_pin};
use dsp::{MeterReader, SampleRate};

use super::clock_gates::{self, BusClock};
use super::fault;
use super::knob_adc;
use super::lcd_link::{LcdLink, TearingEffect};
use super::map::{
    GPIO_DIRCLR, GPIO_DIRCLR_PORT_STEP, GPIO_W, GPIO_W_PIN_STEP, GPIO_W_PORT_STEP, GPIO_W_READ_LOW,
    IOCON_FACTORY_PULLED_UP_INPUT, IOCON_PIO0_2, IOCON_PIO1_15,
};

const NO_BUTTON_PRESSED: ExpanderInputs = ExpanderInputs(0xFF);

#[derive(Clone, Copy)]
pub struct StatusFacts {
    pub rate: SampleRate,
    pub clock: ClockLock,
    pub grant: PhantomGrant,
    pub dsp: DspBadge,
    pub monitoring: [Monitoring; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Completion {
    Nothing,
    StepDone,
    TransferDone,
}

#[derive(Clone, Copy)]
enum Lcd {
    Dark,
    Lit {
        state: scheduler::State,
        awaiting: Completion,
    },
}

#[derive(Clone, Copy)]
enum KnobScan {
    Off,
    Scanning(knob_scan::State),
}

pub enum BusNews {
    Nothing,
    Buttons(ExpanderInputs),
    Fault(FaultCode),
}

pub struct Panel {
    link: LcdLink,
    lcd: Lcd,
    shown: Screen,
    tearing_effect: TearingEffect,
    knob_scan: KnobScan,
    knobs: Option<KnobReadings>,
    buttons: ExpanderInputs,
    bus: panel_bus::State,
    #[cfg(feature = "sound2-ui")]
    glass: Glass,
}

const fn screen_at_boot(previous_boot: Option<Breadcrumb>) -> Screen {
    Screen {
        rate: BOOT_RATE,
        clock: ClockLock::Changing,
        playback: StreamActivity::Idle,
        capture: StreamActivity::Idle,
        phantom: [PhantomPower::Off; 2],
        meters: MeterFrame::SILENT,
        dsp: DspBadge::Bypass,
        #[cfg(feature = "sound2-ui")]
        monitoring: [Monitoring::Off; 2],
        #[cfg(feature = "sound2-ui")]
        pill: PillFade::AT_BOOT,
        #[cfg(feature = "sound2-ui")]
        profile: device::lcd::screen::MeterProfileName::Studio,
        #[cfg(feature = "sound2-ui")]
        reduced_bands: None,
        breadcrumb: Breadcrumb::START,
        previous_boot,
    }
}

// WHY: The amber cue shows only when the band governor runs fewer bands than the preset holds, the 100 MHz fallback at 88.2 and 96 kHz
#[cfg(feature = "sound3")]
fn reduced_bands(facts: StatusFacts) -> Option<u8> {
    let plays_eq = matches!(
        facts.dsp,
        DspBadge::Clarity | DspBadge::Warm | DspBadge::Custom | DspBadge::On
    );
    let running = dsp::bands_running_at(facts.rate.frames_per_tick());
    (plays_eq
        && facts.rate.frames_per_tick() == dsp::FramesPerTick::Twelve
        && running < dsp::PEQ_BANDS)
        .then_some(running as u8)
}

#[cfg(all(feature = "sound2-ui", not(feature = "sound3")))]
fn reduced_bands(_facts: StatusFacts) -> Option<u8> {
    None
}

fn read_pin(pin: Pin) -> u32 {
    GPIO_W
        .bytes_after(
            usize::from(pin.port) * GPIO_W_PORT_STEP + usize::from(pin.number) * GPIO_W_PIN_STEP,
        )
        .read()
}

fn jack(input: Input) -> Jack {
    if read_pin(jack_sense_pin(input)) == GPIO_W_READ_LOW {
        Jack::Removed
    } else {
        Jack::Inserted
    }
}

impl Panel {
    pub fn claim(previous_boot: Option<Breadcrumb>) -> Option<Self> {
        Some(Self {
            link: LcdLink::claim()?,
            lcd: Lcd::Dark,
            shown: screen_at_boot(previous_boot),
            tearing_effect: TearingEffect::High,
            knob_scan: KnobScan::Off,
            knobs: None,
            buttons: NO_BUTTON_PRESSED,
            bus: panel_bus::State::AT_BOOT,
            #[cfg(feature = "sound2-ui")]
            glass: Glass::BLANK,
        })
    }

    pub fn start(&mut self) {
        let Lcd::Dark = self.lcd else {
            return;
        };
        clock_gates::enable(&[BusClock::Iocon, BusClock::Gpio0, BusClock::Gpio1]);
        IOCON_PIO1_15.write(IOCON_FACTORY_PULLED_UP_INPUT);
        IOCON_PIO0_2.write(IOCON_FACTORY_PULLED_UP_INPUT);
        for input in Input::BOTH {
            let pin = jack_sense_pin(input);
            GPIO_DIRCLR
                .bytes_after(usize::from(pin.port) * GPIO_DIRCLR_PORT_STEP)
                .write(1 << pin.number);
        }
        let (state, outputs) = scheduler::start();
        self.lcd = Lcd::Lit {
            state,
            awaiting: Completion::Nothing,
        };
        self.execute_all(outputs.as_slice());
    }

    pub fn knob_scan_started(&mut self) {
        self.knob_scan = KnobScan::Scanning(knob_scan::State::START);
    }

    pub fn fault(&mut self, code: FaultCode) {
        self.step_lcd(scheduler::Event::FaultRaised(code));
    }

    pub fn elapsed(&mut self, elapsed: Milliseconds) {
        self.step_lcd(scheduler::Event::Elapsed(elapsed));
    }

    pub fn is_lit(&self) -> bool {
        matches!(self.lcd, Lcd::Lit { .. })
    }

    pub fn poll(&mut self, facts: StatusFacts, meters: &mut MeterReader<'_>) {
        self.poll_knobs();
        let Lcd::Lit { awaiting, .. } = self.lcd else {
            return;
        };
        if awaiting != Completion::Nothing && self.link.finished() {
            self.set_awaiting(Completion::Nothing);
            self.step_lcd(match awaiting {
                Completion::StepDone => scheduler::Event::StepDone,
                Completion::Nothing | Completion::TransferDone => scheduler::Event::TransferDone,
            });
        }
        let level = LcdLink::tearing_effect();
        let falling = self.tearing_effect == TearingEffect::High && level == TearingEffect::Low;
        self.tearing_effect = level;
        if falling {
            self.frame(facts, meters);
        }
        self.want(PanelTransfer::Leds(ExpanderOutputs::showing(
            facts.grant,
            facts.monitoring,
        )));
    }

    pub fn want(&mut self, transfer: PanelTransfer) {
        self.bus = panel_bus::transition(self.bus, panel_bus::Event::Wanted(transfer)).0;
    }

    pub fn holds_bus(&self) -> bool {
        self.bus.in_flight().is_some()
    }

    pub fn take_free_bus(&mut self) -> Option<Command> {
        let (next, outputs) = panel_bus::transition(self.bus, panel_bus::Event::BusFree);
        self.bus = next;
        outputs.as_slice().iter().find_map(|output| match *output {
            panel_bus::Output::Begin(command) => Some(command),
            panel_bus::Output::ButtonsRead(_) | panel_bus::Output::Fault(_) => None,
        })
    }

    pub fn expander_ready(&mut self) {
        self.bus = panel_bus::transition(self.bus, panel_bus::Event::ExpanderReady).0;
    }

    pub fn bus_preempted(&mut self) {
        self.bus = panel_bus::transition(self.bus, panel_bus::Event::Preempted).0;
    }

    pub fn answered(&mut self, answer: Answer) -> BusNews {
        let (next, outputs) = panel_bus::transition(self.bus, panel_bus::Event::Answered(answer));
        self.bus = next;
        let mut news = BusNews::Nothing;
        for output in outputs.as_slice() {
            match *output {
                panel_bus::Output::ButtonsRead(inputs) => {
                    self.buttons = inputs;
                    news = BusNews::Buttons(inputs);
                }
                panel_bus::Output::Fault(code) => news = BusNews::Fault(code),
                panel_bus::Output::Begin(_) => {}
            }
        }
        news
    }

    pub fn readings(&self) -> Option<PanelReadings> {
        Some(PanelReadings {
            knobs: self.knobs?,
            jacks: Input::BOTH.map(jack),
            buttons: self.buttons,
        })
    }

    pub const fn buttons(&self) -> ExpanderInputs {
        self.buttons
    }

    pub fn jacks(&self) -> [Jack; 2] {
        Input::BOTH.map(jack)
    }

    fn poll_knobs(&mut self) {
        let KnobScan::Scanning(state) = self.knob_scan else {
            return;
        };
        let Some(event) = knob_adc::poll(state.awaiting()) else {
            return;
        };
        let (next, outputs) = knob_scan::transition(state, event);
        self.knob_scan = KnobScan::Scanning(next);
        for output in outputs.as_slice() {
            if let knob_scan::Output::Readings(readings) = *output {
                self.knobs = Some(readings);
            }
            knob_adc::act(*output);
        }
    }

    #[cfg(feature = "sound2-ui")]
    fn frame(&mut self, facts: StatusFacts, meters: &mut MeterReader<'_>) {
        let readings = METER_CHANNELS.map(|channel| meters.take(channel));
        let profile = device::lcd::screen::meter_profile();
        let [_, _, out_l, out_r] = readings;
        #[cfg(feature = "sound3")]
        let tapped = match profile.profile().tap {
            device::lcd::screen::MeterTap::UsbIntake => {
                let [left, right] = meters.take_intake();
                [readings[0], readings[1], left, right]
            }
            device::lcd::screen::MeterTap::Dac => readings,
        };
        #[cfg(not(feature = "sound3"))]
        let tapped = readings;
        let mut screen = Screen {
            rate: facts.rate,
            clock: facts.clock,
            playback: StreamActivity::Idle,
            capture: StreamActivity::Idle,
            phantom: Input::BOTH.map(|input| facts.grant.power(input)),
            monitoring: facts.monitoring,
            meters: self.shown.meters.next_frame_tapped(
                tapped,
                [out_l, out_r],
                meters.take_limited_ticks(),
                profile.profile().release,
            ),
            profile,
            reduced_bands: reduced_bands(facts),
            dsp: facts.dsp,
            pill: self.shown.pill,
            breadcrumb: fault::breadcrumb(),
            previous_boot: self.shown.previous_boot,
        };
        screen.pill = screen
            .pill
            .next_frame(screen.pill_target(), device::lcd::screen::motion());
        let dirty = screen.regions_changed_from(&self.shown);
        self.shown = screen;
        if !dirty.is_empty() {
            self.step_lcd(scheduler::Event::RegionsDirty(dirty));
        }
        self.step_lcd(scheduler::Event::TeEdge);
    }

    #[cfg(not(feature = "sound2-ui"))]
    fn frame(&mut self, facts: StatusFacts, meters: &mut MeterReader<'_>) {
        let screen = Screen {
            rate: facts.rate,
            clock: facts.clock,
            playback: StreamActivity::Idle,
            capture: StreamActivity::Idle,
            phantom: Input::BOTH.map(|input| facts.grant.power(input)),
            meters: self
                .shown
                .meters
                .next_frame(METER_CHANNELS.map(|channel| meters.take(channel))),
            dsp: facts.dsp,
            breadcrumb: fault::breadcrumb(),
            previous_boot: self.shown.previous_boot,
        };
        let dirty = screen.regions_changed_from(&self.shown);
        self.shown = screen;
        if !dirty.is_empty() {
            self.step_lcd(scheduler::Event::RegionsDirty(dirty));
        }
        self.step_lcd(scheduler::Event::TeEdge);
    }

    fn set_awaiting(&mut self, completion: Completion) {
        if let Lcd::Lit { state, .. } = self.lcd {
            self.lcd = Lcd::Lit {
                state,
                awaiting: completion,
            };
        }
    }

    fn step_lcd(&mut self, event: scheduler::Event) {
        let Lcd::Lit { state, awaiting } = self.lcd else {
            return;
        };
        let (next, outputs) = scheduler::transition(state, event);
        self.lcd = Lcd::Lit {
            state: next,
            awaiting,
        };
        self.execute_all(outputs.as_slice());
    }

    fn execute_all(&mut self, outputs: &[scheduler::Output]) {
        for output in outputs {
            let mut immediate = self.execute(*output);
            while let Some(event) = immediate.take() {
                let Lcd::Lit { state, awaiting } = self.lcd else {
                    return;
                };
                let (next, more) = scheduler::transition(state, event);
                self.lcd = Lcd::Lit {
                    state: next,
                    awaiting,
                };
                for output in more.as_slice() {
                    immediate = self.execute(*output);
                }
            }
        }
    }

    fn execute(&mut self, output: scheduler::Output) -> Option<scheduler::Event> {
        match output {
            scheduler::Output::Init(InitAction::ConfigureSpi) => {
                self.link.configure_spi();
                Some(scheduler::Event::StepDone)
            }
            scheduler::Output::Init(InitAction::DriveP1_6High) => {
                self.link.drive_reset_or_backlight_high();
                Some(scheduler::Event::StepDone)
            }
            scheduler::Output::Init(InitAction::BacklightOn) => {
                self.link.drive_backlight_high();
                Some(scheduler::Event::StepDone)
            }
            scheduler::Output::Init(InitAction::Send(command)) => {
                self.link.send_command(command);
                self.set_awaiting(Completion::StepDone);
                None
            }
            scheduler::Output::ClearScreen(colour) => {
                self.link.clear(colour);
                self.set_awaiting(Completion::TransferDone);
                None
            }
            #[cfg(not(feature = "sound2-ui"))]
            scheduler::Output::SendRegion(region) => {
                self.link.send_region(region, &self.shown);
                self.set_awaiting(Completion::TransferDone);
                None
            }
            // WHY: The screen scheduler emits only SendBatch, so a lone region is never asked for
            #[cfg(feature = "sound2-ui")]
            scheduler::Output::SendRegion(_) => Some(scheduler::Event::TransferDone),
            #[cfg(feature = "sound2-ui")]
            scheduler::Output::SendBatch(regions) => {
                let batch = self.link.send_batch(regions, &self.shown, &mut self.glass);
                if batch.count() == 0 {
                    return Some(scheduler::Event::TransferDone);
                }
                self.set_awaiting(Completion::TransferDone);
                (!batch.leftover.is_empty())
                    .then_some(scheduler::Event::RegionsDirty(batch.leftover))
            }
            scheduler::Output::SendFaultCode(code) => {
                self.link.send_fault(code, fault::breadcrumb().place.stage);
                self.set_awaiting(Completion::TransferDone);
                None
            }
        }
    }
}
