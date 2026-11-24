use device::breadcrumb::Breadcrumb;
use device::console::{
    self, BusHealth, ConsoleCommand, ConsoleThread, Output, PanelFacts, Refusal, Reply, Report,
    bus_report, clocks_report, factory_serial_report, fault_report, knob_report, limiter_report,
    panel_report, peak_tenths_of_dbfs, power_report, serial_report,
};
use device::cycles::{CycleSpan, CycleSpans};
use device::knob::Knobs;
use device::usb::SerialNumber;
use device::usb::request::DataTerminal;
use dsp::{
    CoefficientPublisher, DspMode, DspModeSwitch, LimiterSettings, MeterChannel, MeterReader,
    RateFamily, SampleRate,
};
#[cfg(not(feature = "sound2"))]
use dsp::{PeqPreset, PresetBand};
use stream::PlaybackCounterReader;

use super::audio_dma;
use super::clocks;
use super::factory;
use super::fault;
use super::handovers::{self, TICK_CYCLES, USB_CYCLES};
use super::interrupts;
use super::power;

const BYTES_PER_PASS: usize = 64;
const LONGEST_REPLY_BYTES: usize = 192;

type Out<'r> = Reply<'r, device::console::ConsoleOut<'static>>;

pub struct Console {
    #[cfg(feature = "sound2")]
    presets: super::presets::PresetControl,
    session: console::State,
    link: ConsoleThread<'static>,
    #[cfg(not(feature = "sound2"))]
    preset: PeqPreset,
    limiter: LimiterSettings,
    serial: SerialNumber,
    previous_boot: Option<Breadcrumb>,
}

pub struct ConsoleFacts {
    pub rate: SampleRate,
    pub running: bool,
    pub knobs: Option<Knobs>,
    pub panel: PanelFacts,
    pub bus: BusHealth,
}

pub struct ConsoleReach<'r> {
    pub meters: &'r mut MeterReader<'static>,
    pub playback_counters: &'r PlaybackCounterReader<'static>,
    pub coefficients: &'r mut CoefficientPublisher<'static>,
    pub dsp_mode: &'static DspModeSwitch,
}

const fn opened_or_closed(terminal: DataTerminal) -> console::Event {
    match terminal {
        DataTerminal::Present => console::Event::PortOpened,
        DataTerminal::Absent => console::Event::PortClosed,
    }
}

impl Console {
    pub fn new(
        link: ConsoleThread<'static>,
        serial: SerialNumber,
        previous_boot: Option<Breadcrumb>,
    ) -> Self {
        Self {
            #[cfg(feature = "sound2")]
            presets: super::presets::PresetControl::new(),
            session: console::State::Closed,
            link,
            #[cfg(not(feature = "sound2"))]
            preset: PeqPreset::FLAT,
            limiter: LimiterSettings::default(),
            serial,
            previous_boot,
        }
    }

    pub const fn previous_boot(&self) -> Option<Breadcrumb> {
        self.previous_boot
    }

    pub const fn limiter(&self) -> LimiterSettings {
        self.limiter
    }

    pub fn on_data_terminal(&mut self, terminal: DataTerminal) {
        let (next, _) = console::transition(self.session, opened_or_closed(terminal));
        self.session = next;
    }

    #[cfg(feature = "sound2")]
    pub fn badge(&self) -> device::lcd::screen::DspBadge {
        self.presets.badge()
    }

    #[cfg(feature = "sound2")]
    fn presets_reach<'r>(
        facts: &ConsoleFacts,
        reach: &'r mut ConsoleReach<'_>,
    ) -> super::presets::Reach<'r> {
        super::presets::Reach {
            coefficients: reach.coefficients,
            dsp_mode: reach.dsp_mode,
            rate: facts.rate,
            running: facts.running,
        }
    }

    // WHY: A front-panel switch between the clarity and warm presets; a MON 2 press held past one second is never a monitor toggle (phantom.rs HeldTooLong), so it carries the preset toggle
    #[cfg(feature = "sound2")]
    pub fn toggle_preset(
        &mut self,
        coefficients: &mut CoefficientPublisher<'static>,
        dsp_mode: &'static DspModeSwitch,
        rate: SampleRate,
        running: bool,
    ) {
        self.presets.toggle(&mut super::presets::Reach {
            coefficients,
            dsp_mode,
            rate,
            running,
        });
    }

    #[inline(never)]
    pub fn serve(&mut self, facts: &ConsoleFacts, reach: &mut ConsoleReach<'_>) {
        #[cfg(feature = "sound2")]
        self.presets.poll(&mut Self::presets_reach(facts, reach));
        for _ in 0..BYTES_PER_PASS {
            if self.link.to_host.free_bytes() < LONGEST_REPLY_BYTES {
                break;
            }
            let Some(byte) = self.link.from_host.next_byte() else {
                break;
            };
            let (next, outputs) = console::transition(self.session, console::Event::Byte(byte));
            self.session = next;
            for output in outputs.as_slice() {
                self.answer(output.admitted(facts.running), facts, reach);
            }
        }
        self.link.to_host.flush();
    }

    fn answer(&mut self, output: Output, facts: &ConsoleFacts, reach: &mut ConsoleReach<'_>) {
        match output {
            Output::Reject(refusal) => {
                Reply::new(&mut self.link.to_host).text(refusal.reply());
            }
            Output::Execute(command) => self.execute(command, facts, reach),
        }
    }

    #[inline(never)]
    fn execute(
        &mut self,
        command: ConsoleCommand,
        facts: &ConsoleFacts,
        reach: &mut ConsoleReach<'_>,
    ) {
        match command {
            ConsoleCommand::Report(report) => {
                let mut reply = Reply::new(&mut self.link.to_host);
                write_report(
                    &mut reply,
                    report,
                    facts,
                    reach,
                    &self.serial,
                    self.previous_boot,
                );
            }
            #[cfg(not(feature = "sound2"))]
            ConsoleCommand::SetDspMode(mode) => {
                reach.dsp_mode.request(mode);
                TICK_CYCLES.follow_mode(mode);
                USB_CYCLES.follow_mode(mode);
                ok(&mut self.link.to_host);
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::SetDspMode(mode) => {
                let mut presets = Self::presets_reach(facts, reach);
                match mode {
                    DspMode::Bypass => self
                        .presets
                        .choose(device::preset::Preset::Flat, &mut presets),
                    DspMode::On => self.presets.eq_on(&mut presets),
                }
                ok(&mut self.link.to_host);
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::Preset(choice) => {
                if let Some(preset) = choice {
                    self.presets
                        .choose(preset, &mut Self::presets_reach(facts, reach));
                }
                self.presets
                    .report(&mut Reply::new(&mut self.link.to_host), facts.rate);
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::SetTrim(trim) => {
                self.presets
                    .set_trim(trim, &mut Self::presets_reach(facts, reach));
                self.presets
                    .report(&mut Reply::new(&mut self.link.to_host), facts.rate);
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::BeginBands => {
                self.presets.begin();
                ok(&mut self.link.to_host);
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::CommitBands => {
                if self.presets.commit(&mut Self::presets_reach(facts, reach)) {
                    self.presets
                        .report(&mut Reply::new(&mut self.link.to_host), facts.rate);
                } else {
                    Reply::new(&mut self.link.to_host).text(Refusal::MalformedArguments.reply());
                }
            }
            #[cfg(feature = "sound3")]
            ConsoleCommand::MeterProfile(choice) => {
                use device::lcd::screen::{MeterProfileName, meter_profile, set_meter_profile};
                if let Some(profile) = choice {
                    set_meter_profile(profile);
                }
                let shown: &[u8] = match meter_profile() {
                    MeterProfileName::Obs => {
                        b"meter obs tap=usb-intake scale=linear-60 zones=-20,-9 release=11.76db/s"
                    }
                    MeterProfileName::Studio => {
                        b"meter studio tap=dac scale=two-slope zones=-12,-3 release=13.33db/s"
                    }
                };
                let mut reply = Reply::new(&mut self.link.to_host);
                reply.text(shown);
                reply.end();
            }
            #[cfg(feature = "sound2-ui")]
            ConsoleCommand::LcdMotion(choice) => {
                if let Some(on) = choice {
                    device::lcd::screen::set_motion(on);
                }
                let shown: &[u8] = if device::lcd::screen::motion() {
                    b"lcd motion on"
                } else {
                    b"lcd motion off"
                };
                let mut reply = Reply::new(&mut self.link.to_host);
                reply.text(shown);
                reply.end();
            }
            ConsoleCommand::SetBand { band, settings } => {
                self.change_band(band.index(), Some(settings), facts, reach);
            }
            ConsoleCommand::DisableBand(band) => {
                self.change_band(band.index(), None, facts, reach);
            }
            ConsoleCommand::SetLimiter(settings) => {
                self.limiter = settings;
                handovers::retune_limiter(settings, facts.rate.family());
                limiter_report(&mut Reply::new(&mut self.link.to_host), settings);
            }
            ConsoleCommand::ResetMeters => {
                let mut reply = Reply::new(&mut self.link.to_host);
                reply.text(b"meters");
                for (name, channel) in METERED {
                    let reading = reach.meters.take(channel);
                    reply.text(b" ").text(name).text(b"=");
                    match peak_tenths_of_dbfs(reading.peak.to_bits()) {
                        Some(tenths) => reply.tenths(tenths),
                        None => reply.text(b"silent"),
                    };
                    reply.field(b"clips", reading.clipped_ticks.count());
                }
                reply.end();
            }
        }
    }

    #[cfg(feature = "sound2")]
    #[inline(never)]
    fn change_band(
        &mut self,
        index: usize,
        settings: Option<dsp::Band>,
        facts: &ConsoleFacts,
        reach: &mut ConsoleReach<'_>,
    ) {
        if self
            .presets
            .edit_band(index, settings, &mut Self::presets_reach(facts, reach))
        {
            ok(&mut self.link.to_host);
        } else {
            Reply::new(&mut self.link.to_host).text(Refusal::SettingOutOfRange.reply());
        }
    }

    #[cfg(not(feature = "sound2"))]
    #[inline(never)]
    fn change_band(
        &mut self,
        index: usize,
        settings: Option<dsp::Band>,
        facts: &ConsoleFacts,
        reach: &mut ConsoleReach<'_>,
    ) {
        let Some(slot) = self.preset.bands.get_mut(index) else {
            Reply::new(&mut self.link.to_host).text(Refusal::SettingOutOfRange.reply());
            return;
        };
        *slot = match settings {
            Some(band) => PresetBand {
                band,
                enabled: true,
            },
            None => PresetBand {
                enabled: false,
                ..*slot
            },
        };
        reach.coefficients.change(&self.preset, facts.rate);
        ok(&mut self.link.to_host);
    }
}

const METERED: [(&[u8], MeterChannel); 4] = [
    (b"in1", MeterChannel::Input1),
    (b"in2", MeterChannel::Input2),
    (b"outl", MeterChannel::OutputLeft),
    (b"outr", MeterChannel::OutputRight),
];

const MODES: [(&[u8], DspMode); 2] = [(b"bypass", DspMode::Bypass), (b"dsp", DspMode::On)];

const FAMILIES: [(&[u8], RateFamily); 2] = [
    (b"44k1", RateFamily::FortyFourPointOneKilohertz),
    (b"48k", RateFamily::FortyEightKilohertz),
];

fn ok(out: &mut device::console::ConsoleOut<'static>) {
    Reply::new(out).text(b"ok\r\n");
}

#[inline(never)]
fn cycles_line(reply: &mut Out<'_>, name: &[u8], spans: &CycleSpans) {
    for (family_name, family) in FAMILIES {
        for (mode_name, mode) in MODES {
            let CycleSpan { last, most } = spans.read_in(family, mode);
            reply
                .text(b"cycles ")
                .text(name)
                .text(b" ")
                .text(family_name)
                .text(b" ")
                .text(mode_name)
                .field(b"last", last)
                .field(b"max", most)
                .end();
        }
    }
}

#[inline(never)]
fn write_report(
    reply: &mut Out<'_>,
    report: Report,
    facts: &ConsoleFacts,
    reach: &mut ConsoleReach<'_>,
    serial: &SerialNumber,
    previous_boot: Option<Breadcrumb>,
) {
    match report {
        Report::Status => {
            reply
                .text(b"status")
                .field(b"rate", facts.rate.hz())
                .text(match reach.dsp_mode.requested() {
                    DspMode::On => b" dsp=on",
                    DspMode::Bypass => b" dsp=bypass",
                })
                .text(if facts.running {
                    b" stage=running"
                } else {
                    b" stage=booting"
                })
                .end();
        }
        Report::Counters => {
            let counters = reach.playback_counters.take();
            let (lowest, highest) = counters
                .effective_fill_while_running
                .map_or((0, 0), |fill| (fill.lowest.count(), fill.highest.count()));
            reply
                .text(b"counters")
                .field(b"underruns", counters.underruns)
                .field(b"overruns", counters.overruns)
                .field(b"fill_min", lowest)
                .field(b"fill_max", highest)
                .end();
        }
        Report::Dma => {
            reply
                .text(b"dma")
                .field(b"ticks", u32::from(audio_dma::ticks_served()))
                .field(
                    b"usb_interrupts",
                    u32::from(interrupts::usb_interrupts_served()),
                )
                .end();
        }
        Report::Cycles => {
            cycles_line(reply, b"tick", &TICK_CYCLES);
            cycles_line(reply, b"usb", &USB_CYCLES);
        }
        Report::Fault => fault_report(reply, fault::breadcrumb(), previous_boot),
        Report::Version => {
            reply
                .text(b"version m2x-fw ")
                .text(env!("CARGO_PKG_VERSION").as_bytes())
                .end();
        }
        Report::Knob => knob_report(reply, facts.knobs),
        Report::Serial => match factory::read_on_demand() {
            Some(factory) => factory_serial_report(reply, serial, factory),
            None => serial_report(reply, serial),
        },
        Report::Panel => panel_report(reply, facts.panel),
        Report::Clocks => clocks_report(reply, clocks::facts()),
        Report::Bus => bus_report(reply, facts.bus),
        Report::Power => power_report(reply, power::facts()),
        #[cfg(feature = "sound3")]
        Report::PowerNmpa => device::console::power_nmpa_report(
            reply,
            device::console::NmpaFacts {
                nmpa: factory::nmpa(),
                planned: power::planned_voltage(),
                core: clocks::speed::clock(),
            },
        ),
    }
}
