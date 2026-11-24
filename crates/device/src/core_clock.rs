// WHY: M 25 over N 2 from the 24 MHz crystal is a 300 MHz CCO, inside UM11126's 275 to 550 MHz, and P 1 halves it to 150.000 MHz
pub const PLL1_NDEC_FOR_150_MHZ: u32 = 2;
pub const CORE_150_MHZ: u32 = 150_000_000;
pub const SYSTICK_RELOAD_AT_150_MHZ: u32 = CORE_150_MHZ / 1_000 - 1;
pub const CORE_150_MHZ_TICK_CYCLES: u32 = CORE_150_MHZ / 8_000;

// WHY: NXP fsl_power.c lowpower_set_dcdc_power_profile and lowpower_get_part_process_corner, the HIGH profile DCDC trims and the two PVT ring-oscillator trims, bit 0 valid and the value above it
pub const NMPA_ADDRESSES: [u32; 4] = [0x0009_FCD8, 0x0009_FCDC, 0x0009_FD30, 0x0009_FD40];

const VALID: u32 = 1;
const NOMINAL_RING_HZ: u32 = 19_300_000;
const NOMINAL_RING_SPREAD_HZ: u32 = 6 * 400_000;

const VOUT_SHIFT: u32 = 17;
const VOUT_MASK: u32 = 0xF << VOUT_SHIFT;
const VADJ_MASK: u32 = 0x1F;
const BOOST_SHIFT: u32 = 10;
const BOOST_MASK: u32 = 0x1F << BOOST_SHIFT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NmpaWords {
    pub dcdc_high: [u32; 2],
    pub ringo: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nmpa {
    Unreadable,
    Read(NmpaWords),
}

impl NmpaWords {
    pub const fn from_words(words: [u32; 4]) -> Self {
        Self {
            dcdc_high: [words[0], words[1]],
            ringo: [words[2], words[3]],
        }
    }

    pub const fn words(self) -> [u32; 4] {
        [
            self.dcdc_high[0],
            self.dcdc_high[1],
            self.ringo[0],
            self.ringo[1],
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PmcWords {
    pub dcdc0: u32,
    pub dcdc1: u32,
    pub ldopmu: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    Slow,
    Nominal,
    Fast,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoltageSource {
    Nmpa,
    NmpaInvalid,
    NmpaUnreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoltagePlan {
    pub corner: Corner,
    pub source: VoltageSource,
    pub millivolts: u32,
    pub writes: PmcWords,
}

struct Row {
    millivolts: u32,
    vout: u32,
    vadj: u32,
    boost: u32,
}

// WHY: NXP fsl_power.c VOLTAGE_*_HIG_MV and lowpower_set_system_voltage, the HIGH profile row per process corner, as V_DCDC and V_AO codes
const fn row(corner: Corner) -> Row {
    match corner {
        Corner::Slow => Row {
            millivolts: 1200,
            vout: 10,
            vadj: 30,
            boost: 31,
        },
        Corner::Nominal => Row {
            millivolts: 1150,
            vout: 8,
            vadj: 30,
            boost: 31,
        },
        Corner::Fast => Row {
            millivolts: 1050,
            vout: 4,
            vadj: 20,
            boost: 25,
        },
    }
}

const fn ring_hz(word: u32) -> Option<u32> {
    if word & VALID != 0 {
        Some(word >> 1)
    } else {
        None
    }
}

// WHY: NXP lowpower_get_part_process_corner takes the slower ring and counts an invalid one as 19.3 MHz; with no valid ring at all this image takes the slow corner, the highest voltage NXP uses
const fn corner_of(words: NmpaWords) -> Option<Corner> {
    let (first, second) = (ring_hz(words.ringo[0]), ring_hz(words.ringo[1]));
    let slower = match (first, second) {
        (None, None) => return None,
        (Some(a), Some(b)) => {
            if a < b {
                a
            } else {
                b
            }
        }
        (Some(a), None) | (None, Some(a)) => {
            if a < NOMINAL_RING_HZ {
                a
            } else {
                NOMINAL_RING_HZ
            }
        }
    };
    Some(if slower <= NOMINAL_RING_HZ - NOMINAL_RING_SPREAD_HZ {
        Corner::Slow
    } else if slower <= NOMINAL_RING_HZ + NOMINAL_RING_SPREAD_HZ {
        Corner::Nominal
    } else {
        Corner::Fast
    })
}

const fn higher(now: u32, mask: u32, shift: u32, code: u32) -> u32 {
    let held = (now & mask) >> shift;
    let raised = if code > held { code } else { held };
    (now & !mask) | (raised << shift)
}

// WHY: NXP POWER_SetVoltageForFreq(150 MHz): the HIGH DCDC trims when valid, then the corner's LDOPMU VADJ and VADJ_BOOST and DCDC0 VOUT; every field is raised to the planned code and never lowered, so a fast part keeps the 1.05 V and 1.10 V it already runs at
pub const fn plan_for_150_mhz(nmpa: Nmpa, now: PmcWords) -> VoltagePlan {
    let (corner, source, trims) = match nmpa {
        Nmpa::Unreadable => (Corner::Slow, VoltageSource::NmpaUnreadable, None),
        Nmpa::Read(words) => {
            let trims = if words.dcdc_high[0] & VALID != 0 {
                Some([words.dcdc_high[0] >> 1, words.dcdc_high[1]])
            } else {
                None
            };
            match (corner_of(words), trims) {
                (Some(corner), _) => (corner, VoltageSource::Nmpa, trims),
                (None, Some(_)) => (Corner::Slow, VoltageSource::Nmpa, trims),
                (None, None) => (Corner::Slow, VoltageSource::NmpaInvalid, None),
            }
        }
    };
    let target = row(corner);
    let (dcdc0, dcdc1) = match trims {
        Some([trim0, trim1]) => ((trim0 & !VOUT_MASK) | (now.dcdc0 & VOUT_MASK), trim1),
        None => (now.dcdc0, now.dcdc1),
    };
    let ldopmu = higher(now.ldopmu, VADJ_MASK, 0, target.vadj);
    VoltagePlan {
        corner,
        source,
        millivolts: target.millivolts,
        writes: PmcWords {
            dcdc0: higher(dcdc0, VOUT_MASK, VOUT_SHIFT, target.vout),
            dcdc1,
            ldopmu: higher(ldopmu, BOOST_MASK, BOOST_SHIFT, target.boost),
        },
    }
}

const fn field(word: u32, mask: u32, shift: u32) -> u32 {
    (word & mask) >> shift
}

pub const fn voltage_reached(plan: &VoltagePlan, read_back: Option<PmcWords>) -> bool {
    let Some(now) = read_back else {
        return false;
    };
    let want = plan.writes;
    field(now.dcdc0, VOUT_MASK, VOUT_SHIFT) >= field(want.dcdc0, VOUT_MASK, VOUT_SHIFT)
        && field(now.ldopmu, VADJ_MASK, 0) >= field(want.ldopmu, VADJ_MASK, 0)
        && field(now.ldopmu, BOOST_MASK, BOOST_SHIFT) >= field(want.ldopmu, BOOST_MASK, BOOST_SHIFT)
}

pub const fn ldo_millivolts(ldopmu: u32) -> u32 {
    let code = ldopmu & VADJ_MASK;
    if code >= 10 {
        960 + (code - 10) * 10
    } else {
        675 + code * 25
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreClock {
    Pll150,
    Pll100(HeldAt100),
    Fro96,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldAt100 {
    NotYetRaised,
    VoltageNotConfirmed,
    Pll150Unlocked,
}

impl CoreClock {
    pub const fn hertz(self) -> u32 {
        match self {
            CoreClock::Pll150 => CORE_150_MHZ,
            CoreClock::Pll100(_) => 100_000_000,
            CoreClock::Fro96 => 96_000_000,
        }
    }

    pub const fn tick_cycles(self) -> u32 {
        self.hertz() / 8_000
    }

    pub const fn megahertz_text(self) -> &'static [u8] {
        match self {
            CoreClock::Pll150 => b"150MHz",
            CoreClock::Pll100(_) => b"100MHz",
            CoreClock::Fro96 => b"96MHz",
        }
    }

    pub const fn reason_text(self) -> &'static [u8] {
        match self {
            CoreClock::Pll150 => b"raised",
            CoreClock::Pll100(HeldAt100::NotYetRaised) => b"not-yet-raised",
            CoreClock::Pll100(HeldAt100::VoltageNotConfirmed) => b"voltage-not-confirmed",
            CoreClock::Pll100(HeldAt100::Pll150Unlocked) => b"pll1-150-unlocked",
            CoreClock::Fro96 => b"pll1-unlocked",
        }
    }
}
