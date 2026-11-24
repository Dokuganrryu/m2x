use super::line::{Line, Words};
use super::number::{decimal, tenths, whole_number};
use super::refusal::Refusal;
use dsp::{
    Band, BandFrequency, BandGain, BandParameterError, BandQ, Ceiling, DspMode, FilterKind,
    LimiterSettings, PEQ_BANDS, ReleaseTime, SettingOutOfRange,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Status,
    Counters,
    Dma,
    Knob,
    Serial,
    Version,
    Cycles,
    Fault,
    Panel,
    Clocks,
    Bus,
    Power,
    #[cfg(feature = "sound3")]
    PowerNmpa,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandNumber(u8);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConsoleCommand {
    Report(Report),
    SetDspMode(DspMode),
    SetBand {
        band: BandNumber,
        settings: Band,
    },
    DisableBand(BandNumber),
    SetLimiter(LimiterSettings),
    ResetMeters,
    #[cfg(feature = "sound2")]
    Preset(Option<crate::preset::Preset>),
    #[cfg(feature = "sound2")]
    SetTrim(dsp::PresetTrim),
    #[cfg(feature = "sound2")]
    BeginBands,
    #[cfg(feature = "sound2")]
    CommitBands,
    #[cfg(feature = "screen2")]
    LcdMotion(Option<bool>),
    #[cfg(feature = "sound3")]
    MeterProfile(Option<crate::lcd::screen::MeterProfileName>),
    #[cfg(feature = "register-zero-protocol")]
    RegisterZero(crate::register_zero::OperatorStep),
}

const FIRST_BAND_NUMBER: u32 = 1;

impl BandNumber {
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    fn parse(word: &[u8]) -> Result<Self, Refusal> {
        let index = whole_number(word)?
            .checked_sub(FIRST_BAND_NUMBER)
            .and_then(|index| u8::try_from(index).ok())
            .filter(|&index| usize::from(index) < PEQ_BANDS)
            .ok_or(Refusal::SettingOutOfRange)?;
        Ok(Self(index))
    }
}

impl From<BandParameterError> for Refusal {
    fn from(_: BandParameterError) -> Self {
        Self::SettingOutOfRange
    }
}

impl From<SettingOutOfRange> for Refusal {
    fn from(_: SettingOutOfRange) -> Self {
        Self::SettingOutOfRange
    }
}

impl ConsoleCommand {
    pub const fn only_reads(self) -> bool {
        match self {
            Self::Report(_) => true,
            Self::SetDspMode(_)
            | Self::SetBand { .. }
            | Self::DisableBand(_)
            | Self::SetLimiter(_)
            | Self::ResetMeters => false,
            #[cfg(feature = "sound2")]
            Self::Preset(None) => true,
            #[cfg(feature = "sound2")]
            Self::Preset(Some(_)) | Self::SetTrim(_) | Self::BeginBands | Self::CommitBands => {
                false
            }
            #[cfg(feature = "screen2")]
            Self::LcdMotion(None) => true,
            #[cfg(feature = "screen2")]
            Self::LcdMotion(Some(_)) => false,
            #[cfg(feature = "sound3")]
            Self::MeterProfile(None) => true,
            #[cfg(feature = "sound3")]
            Self::MeterProfile(Some(_)) => false,
            #[cfg(feature = "register-zero-protocol")]
            Self::RegisterZero(_) => false,
        }
    }

    pub(crate) fn parse(line: &Line) -> Result<Self, Refusal> {
        let mut words = line.words();
        let command = match words.next().ok_or(Refusal::NotAllowlisted)? {
            b"status" => Self::Report(Report::Status),
            b"counters" => Self::Report(Report::Counters),
            b"dma" => Self::Report(Report::Dma),
            b"knob" => Self::Report(Report::Knob),
            b"serial" => Self::Report(Report::Serial),
            b"version" => Self::Report(Report::Version),
            b"cycles" => Self::Report(Report::Cycles),
            b"fault" => Self::Report(Report::Fault),
            b"panel" => Self::Report(Report::Panel),
            b"clocks" => Self::Report(Report::Clocks),
            b"i2c" => Self::Report(Report::Bus),
            #[cfg(not(feature = "sound3"))]
            b"power" => Self::Report(Report::Power),
            #[cfg(feature = "sound3")]
            b"power" => match words.next() {
                None => return Ok(Self::Report(Report::Power)),
                Some(b"nmpa") => Self::Report(Report::PowerNmpa),
                Some(_) => return Err(Refusal::MalformedArguments),
            },
            b"dsp" => match words.next() {
                Some(b"bypass") => Self::SetDspMode(DspMode::Bypass),
                Some(b"on") => Self::SetDspMode(DspMode::On),
                _ => return Err(Refusal::NotAllowlisted),
            },
            #[cfg(not(feature = "sound3"))]
            b"meter" => match words.next() {
                Some(b"reset") => Self::ResetMeters,
                _ => return Err(Refusal::NotAllowlisted),
            },
            #[cfg(feature = "sound3")]
            b"meter" => match words.next() {
                Some(b"reset") => Self::ResetMeters,
                None => Self::MeterProfile(None),
                Some(word) => Self::MeterProfile(Some(
                    crate::lcd::screen::MeterProfileName::named(word)
                        .ok_or(Refusal::MalformedArguments)?,
                )),
            },
            b"peq" => match argument(&mut words)? {
                b"off" => Self::DisableBand(BandNumber::parse(argument(&mut words)?)?),
                #[cfg(feature = "sound2")]
                b"begin" => Self::BeginBands,
                #[cfg(feature = "sound2")]
                b"commit" => Self::CommitBands,
                number => Self::SetBand {
                    band: BandNumber::parse(number)?,
                    settings: band_settings(&mut words)?,
                },
            },
            b"lim" => Self::SetLimiter(limiter_settings(&mut words)?),
            #[cfg(feature = "sound2")]
            b"preset" => Self::Preset(match words.next() {
                None => None,
                Some(name) => {
                    Some(crate::preset::Preset::named(name).ok_or(Refusal::MalformedArguments)?)
                }
            }),
            #[cfg(feature = "sound2")]
            b"trim" => Self::SetTrim(dsp::PresetTrim::from_db(decimal(argument(&mut words)?)?)?),
            #[cfg(feature = "screen2")]
            b"lcd" => match words.next() {
                Some(b"motion") => Self::LcdMotion(match words.next() {
                    None => None,
                    Some(b"on") => Some(true),
                    Some(b"off") => Some(false),
                    Some(_) => return Err(Refusal::MalformedArguments),
                }),
                _ => return Err(Refusal::NotAllowlisted),
            },
            #[cfg(feature = "register-zero-protocol")]
            b"register0" => Self::RegisterZero(super::protocol::operator_step(&mut words)?),
            _ => return Err(Refusal::NotAllowlisted),
        };
        match words.next() {
            Some(_) => Err(Refusal::MalformedArguments),
            None => Ok(command),
        }
    }
}

fn argument<'line>(words: &mut Words<'line>) -> Result<&'line [u8], Refusal> {
    words.next().ok_or(Refusal::MalformedArguments)
}

fn filter_kind(word: &[u8]) -> Result<FilterKind, Refusal> {
    match word {
        b"peak" => Ok(FilterKind::Peaking),
        b"lowshelf" => Ok(FilterKind::LowShelf),
        b"highshelf" => Ok(FilterKind::HighShelf),
        b"highpass" => Ok(FilterKind::HighPass),
        b"lowpass" => Ok(FilterKind::LowPass),
        _ => Err(Refusal::MalformedArguments),
    }
}

fn band_settings(words: &mut Words<'_>) -> Result<Band, Refusal> {
    let kind = filter_kind(argument(words)?)?;
    let hz = decimal(argument(words)?)?;
    let db = decimal(argument(words)?)?;
    let q = decimal(argument(words)?)?;
    Ok(Band {
        kind,
        frequency: BandFrequency::from_hz(hz)?,
        gain: BandGain::from_db(db)?,
        q: BandQ::new(q)?,
    })
}

fn limiter_settings(words: &mut Words<'_>) -> Result<LimiterSettings, Refusal> {
    let ceiling_tenths = tenths(argument(words)?)?;
    let release_millis = whole_number(argument(words)?)?;
    let ceiling_tenths = i16::try_from(ceiling_tenths).map_err(|_| Refusal::SettingOutOfRange)?;
    let release_millis = u16::try_from(release_millis).map_err(|_| Refusal::SettingOutOfRange)?;
    Ok(LimiterSettings {
        ceiling: Ceiling::from_tenths_of_dbfs(ceiling_tenths)?,
        release: ReleaseTime::from_millis(release_millis)?,
    })
}
