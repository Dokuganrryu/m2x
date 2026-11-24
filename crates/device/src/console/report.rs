use super::reply::{ByteOut, Reply};
use crate::breadcrumb::Breadcrumb;
use crate::factory::{FACTORY_MAGIC, FactoryRead};
use crate::i2c::Answer;
use crate::knob::{Knob, Knobs};
use crate::monitor::Monitoring;
use crate::phantom::{PhantomPower, PinLevel};
use crate::usb::SerialNumber;
use dsp::LimiterSettings;

const KNOB_NAMES: [(&[u8], Knob); 5] = [
    (b"pre1", Knob::PreampOne),
    (b"pre2", Knob::PreampTwo),
    (b"main", Knob::Main),
    (b"hp", Knob::Headphone),
    (b"mix", Knob::MonitorMix),
];

pub fn knob_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, knobs: Option<Knobs>) {
    reply.text(b"knob");
    match knobs {
        Some(knobs) => {
            for (name, knob) in KNOB_NAMES {
                reply.field(name, u32::from(knobs.position(knob).value()));
            }
        }
        None => {
            reply.text(b" unscanned");
        }
    }
    reply.end();
}

const NOW: [&[u8]; 3] = [b"at", b"code", b"latest"];
const PREVIOUS: [&[u8]; 3] = [b"prev_at", b"prev_code", b"prev_latest"];

#[inline(never)]
fn breadcrumb_fields<Out: ByteOut>(
    reply: &mut Reply<'_, Out>,
    [at, code, latest]: [&[u8]; 3],
    crumb: Breadcrumb,
) {
    reply
        .field(at, u32::from(crumb.place.stage))
        .text(b".")
        .unsigned(u32::from(crumb.place.step))
        .field(code, u32::from(crumb.code))
        .field(latest, u32::from(crumb.latest));
}

pub fn fault_report<Out: ByteOut>(
    reply: &mut Reply<'_, Out>,
    now: Breadcrumb,
    previous: Option<Breadcrumb>,
) {
    reply.text(b"fault");
    breadcrumb_fields(reply, NOW, now);
    match previous {
        Some(previous) => breadcrumb_fields(reply, PREVIOUS, previous),
        None => {
            reply.text(b" prev=none");
        }
    }
    reply.end();
}

pub fn serial_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, serial: &SerialNumber) {
    reply.text(b"serial ").text(&serial.ascii()).end();
}

pub fn limiter_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, settings: LimiterSettings) {
    reply
        .text(b"limiter ceiling=")
        .tenths(i32::from(settings.ceiling.tenths_of_dbfs()))
        .field(b"release", u32::from(settings.release.millis()))
        .end();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelFacts {
    pub buttons: u8,
    pub leds: u8,
    pub phantom: [PhantomPower; 2],
    pub monitoring: [Monitoring; 2],
    pub phantom_rail: Option<PinLevel>,
    pub output_connect: Option<PinLevel>,
}

const fn on_or_off(on: bool) -> &'static [u8] {
    if on { b"on" } else { b"off" }
}

const fn level(level: Option<PinLevel>) -> &'static [u8] {
    match level {
        Some(PinLevel::Low) => b"low",
        Some(PinLevel::High) => b"high",
        None => b"unclocked",
    }
}

pub fn panel_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, facts: PanelFacts) {
    let [phantom_one, phantom_two] = facts.phantom;
    let [monitor_one, monitor_two] = facts.monitoring;
    reply
        .text(b"panel")
        .field(b"buttons", u32::from(facts.buttons))
        .field(b"leds", u32::from(facts.leds));
    for (name, on) in [
        (b"p48_1" as &[u8], phantom_one == PhantomPower::On),
        (b"p48_2", phantom_two == PhantomPower::On),
        (b"mon_1", monitor_one == Monitoring::On),
        (b"mon_2", monitor_two == Monitoring::On),
    ] {
        reply.text(b" ").text(name).text(b"=").text(on_or_off(on));
    }
    reply
        .text(b" p1_4=")
        .text(level(facts.phantom_rail))
        .text(b" p0_28=")
        .text(level(facts.output_connect))
        .end();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainClock {
    Fro12,
    Fro96,
    Pll1,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    Locked,
    Unlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockFacts {
    pub main: MainClock,
    pub core_pll: Lock,
    pub audio_pll: Lock,
    #[cfg(feature = "sound3")]
    pub core: crate::core_clock::CoreClock,
}

const fn lock(lock: Lock) -> &'static [u8] {
    match lock {
        Lock::Locked => b"locked",
        Lock::Unlocked => b"unlocked",
    }
}

pub fn clocks_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, facts: ClockFacts) {
    reply
        .text(b"clocks main=")
        .text(match facts.main {
            MainClock::Fro12 => b"fro12",
            MainClock::Fro96 => b"fro96",
            MainClock::Pll1 => b"pll1",
            MainClock::Other => b"other",
        })
        .text(b" pll1=")
        .text(lock(facts.core_pll))
        .text(b" pll0=")
        .text(lock(facts.audio_pll));
    #[cfg(feature = "sound3")]
    reply
        .text(b" core=")
        .text(facts.core.megahertz_text())
        .text(b" reason=")
        .text(facts.core.reason_text())
        .field(b"tick_cycles", facts.core.tick_cycles());
    reply.end();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusHealth {
    pub answered: u32,
    pub unanswered: u32,
    pub last_unanswered: Option<u8>,
    pub absent: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    Transfer,
    Probe,
}

impl BusHealth {
    pub const START: Self = Self {
        answered: 0,
        unanswered: 0,
        last_unanswered: None,
        absent: 0,
    };

    // WHY: Both ADC addresses are probed and the one that answers is kept, so a probe nobody answers is how a part is found absent, not a bus fault
    pub const fn after(self, peer: u8, asked: Asked, answer: Answer) -> Self {
        match (answer, asked) {
            (Answer::Acked | Answer::Read(_), _) => Self {
                answered: self.answered.saturating_add(1),
                ..self
            },
            (Answer::Nacked, Asked::Probe) => Self {
                absent: self.absent.saturating_add(1),
                ..self
            },
            (Answer::Nacked, Asked::Transfer) => Self {
                unanswered: self.unanswered.saturating_add(1),
                last_unanswered: Some(peer),
                ..self
            },
        }
    }
}

pub fn bus_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, health: BusHealth) {
    reply
        .text(b"i2c")
        .field(b"answered", health.answered)
        .field(b"unanswered", health.unanswered);
    match health.last_unanswered {
        Some(peer) => reply.field(b"last_unanswered", u32::from(peer)),
        None => reply.text(b" last_unanswered=none"),
    };
    reply.field(b"absent_probes", health.absent).end();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerFacts {
    pub dcdc0: Option<u32>,
    pub dcdc1: Option<u32>,
    pub ldopmu: Option<u32>,
}

// WHY: LPC55S28.h PMC_DCDC0_VOUT, bits 20:17, 0.95 V plus 25 mV per step up to 1.2 V at 10
const VOUT_SHIFT: u32 = 17;
const VOUT_MASK: u32 = 0xF;
const VOUT_BASE_MILLIVOLTS: u32 = 950;
const VOUT_STEP_MILLIVOLTS: u32 = 25;

pub const fn dcdc_millivolts(dcdc0: u32) -> u32 {
    VOUT_BASE_MILLIVOLTS + VOUT_STEP_MILLIVOLTS * ((dcdc0 >> VOUT_SHIFT) & VOUT_MASK)
}

fn word<Out: ByteOut>(reply: &mut Reply<'_, Out>, name: &[u8], value: Option<u32>) {
    reply.text(b" ").text(name).text(b"=");
    match value {
        Some(value) => reply.hex(value),
        None => reply.text(b"unreadable"),
    };
}

pub fn power_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, facts: PowerFacts) {
    reply.text(b"power");
    word(reply, b"dcdc0", facts.dcdc0);
    if let Some(dcdc0) = facts.dcdc0 {
        reply.field(b"vout_mv", dcdc_millivolts(dcdc0));
    }
    word(reply, b"dcdc1", facts.dcdc1);
    word(reply, b"ldopmu", facts.ldopmu);
    reply.end();
}

pub fn factory_serial_report<Out: ByteOut>(
    reply: &mut Reply<'_, Out>,
    usb: &SerialNumber,
    factory: FactoryRead,
) {
    reply
        .text(b"serial usb=")
        .text(&usb.ascii())
        .text(b" factory=");
    match factory {
        FactoryRead::Unreadable => {
            reply.text(b"unreadable");
        }
        FactoryRead::Loaded {
            magic: FACTORY_MAGIC,
            serial,
        } => {
            for byte in serial {
                reply.text(&[if byte.is_ascii_graphic() { byte } else { b'?' }]);
            }
        }
        FactoryRead::Loaded { magic, .. } => {
            reply.text(b"blank magic=").hex(magic);
        }
    }
    reply.end();
}

#[cfg(feature = "sound3")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NmpaFacts {
    pub nmpa: crate::core_clock::Nmpa,
    pub planned: Option<(
        crate::core_clock::Corner,
        crate::core_clock::VoltageSource,
        u32,
    )>,
    pub core: crate::core_clock::CoreClock,
}

// WHY: The four NMPA words with their valid bits, the corner and voltage the raise planned and the clock it reached, all from the copies taken at boot, so the report reads no flash
#[cfg(feature = "sound3")]
pub fn power_nmpa_report<Out: ByteOut>(reply: &mut Reply<'_, Out>, facts: NmpaFacts) {
    use crate::core_clock::{Corner, Nmpa, VoltageSource};
    reply.text(b"power nmpa");
    match facts.nmpa {
        Nmpa::Unreadable => {
            reply.text(b" words=unreadable");
        }
        Nmpa::Read(words) => {
            let names: [&[u8]; 4] = [b"dcdc_high0", b"dcdc_high1", b"ringo0", b"ringo1"];
            for (name, value) in names.into_iter().zip(words.words()) {
                reply.text(b" ").text(name).text(b"=").hex(value);
            }
            reply
                .field(b"trim_valid", words.dcdc_high[0] & 1)
                .field(b"ringo0_valid", words.ringo[0] & 1)
                .field(b"ringo1_valid", words.ringo[1] & 1);
        }
    }
    match facts.planned {
        Some((corner, source, millivolts)) => {
            reply
                .text(b" corner=")
                .text(match corner {
                    Corner::Slow => b"sss",
                    Corner::Nominal => b"nnn",
                    Corner::Fast => b"fff",
                })
                .text(b" source=")
                .text(match source {
                    VoltageSource::Nmpa => b"nmpa".as_slice(),
                    VoltageSource::NmpaInvalid => b"nmpa-invalid-sss",
                    VoltageSource::NmpaUnreadable => b"nmpa-unreadable-sss",
                })
                .field(b"plan_mv", millivolts);
        }
        None => {
            reply.text(b" plan=none");
        }
    }
    reply
        .text(b" core=")
        .text(facts.core.megahertz_text())
        .text(b" reason=")
        .text(facts.core.reason_text())
        .end();
}
