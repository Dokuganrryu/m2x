use device::console::{ByteOut, Reply};
use device::lcd::screen::DspBadge;
use device::preset::{Bands, Preset};
use dsp::{
    CROSSFADE_TICKS, CoefficientPublisher, DspMode, DspModeSwitch, PeqPreset, PresetBand,
    PresetTrim, SampleRate, bands_running_at,
};

use super::audio_dma;
use super::handovers::{TICK_CYCLES, USB_CYCLES};

// WHY: Dsp crossfade.rs, a fade is CROSSFADE_TICKS ticks; two more so the tick has adopted the bypass mix before the coefficients move, so no preset or trim change is heard as a step
const DIP_TICKS: u16 = CROSSFADE_TICKS as u16 + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Showing {
    Builtin(Preset),
    Custom,
}

impl Showing {
    const fn plays_eq(self) -> bool {
        !matches!(self, Self::Builtin(Preset::Flat))
    }

    pub const fn badge(self) -> DspBadge {
        match self {
            Self::Builtin(preset) => preset.badge(),
            Self::Custom => DspBadge::Custom,
        }
    }
}

// WHY: Three 560-byte copies of a 14-band preset here took the stack to its floor in the emulator; built-ins stay in their statics and one copy holds custom bands, edited in place while staging
pub struct PresetControl {
    showing: Showing,
    last_eq: Showing,
    builtin: Option<&'static Bands>,
    custom: PeqPreset,
    trim: PresetTrim,
    staging: bool,
    dipping_since: Option<u16>,
    started: bool,
    #[cfg(feature = "sound3")]
    raw_above_96k: bool,
}

pub struct Reach<'r> {
    pub coefficients: &'r mut CoefficientPublisher<'static>,
    pub dsp_mode: &'static DspModeSwitch,
    pub rate: SampleRate,
    pub running: bool,
}

fn nearest_tenth(db: f64) -> i32 {
    let tenths = db * 10.0;
    (if tenths < 0.0 {
        tenths - 0.5
    } else {
        tenths + 0.5
    }) as i32
}

fn follow(dsp_mode: &DspModeSwitch, mode: DspMode) {
    dsp_mode.request(mode);
    TICK_CYCLES.follow_mode(mode);
    USB_CYCLES.follow_mode(mode);
}

impl PresetControl {
    pub const fn new() -> Self {
        Self {
            showing: Showing::Builtin(Preset::Flat),
            last_eq: Showing::Builtin(Preset::AT_BOOT),
            builtin: None,
            custom: PeqPreset::FLAT,
            trim: PresetTrim::NONE,
            staging: false,
            dipping_since: None,
            started: false,
            #[cfg(feature = "sound3")]
            raw_above_96k: false,
        }
    }

    #[cfg(not(feature = "sound3"))]
    pub const fn badge(&self) -> DspBadge {
        self.showing.badge()
    }

    // WHY: Above 96 kHz the device really is bit-perfect, so the pill says RAW
    #[cfg(feature = "sound3")]
    pub const fn badge(&self) -> DspBadge {
        if self.raw_above_96k {
            DspBadge::Raw
        } else {
            self.showing.badge()
        }
    }

    // WHY: 14 bands at 24 frames would pass the cycle cap, and 9 bands miss the curve by 1.6 dB, so at 176.4 and 192 kHz a preset that plays EQ fades to the bit-perfect bypass, and returns to its EQ when the rate comes back down
    #[cfg(feature = "sound3")]
    fn follow_rate(&mut self, reach: &mut Reach<'_>) {
        let above_96k = reach.rate.frames_per_tick() == dsp::FramesPerTick::TwentyFour;
        if above_96k == self.raw_above_96k {
            return;
        }
        self.raw_above_96k = above_96k;
        if !self.showing.plays_eq() {
            return;
        }
        if above_96k {
            self.dipping_since = None;
            follow(reach.dsp_mode, DspMode::Bypass);
        } else {
            self.change(reach);
        }
    }

    #[cfg(feature = "sound3")]
    fn high_rate(bands: &'static Bands) -> (&'static PeqPreset, PresetTrim) {
        bands.for_twelve_frame_bands(bands_running_at(dsp::FramesPerTick::Twelve))
    }

    fn preset(&self) -> &PeqPreset {
        match self.builtin {
            Some(bands) => &bands.preset,
            None => &self.custom,
        }
    }

    pub fn poll(&mut self, reach: &mut Reach<'_>) {
        if !self.started {
            self.started = true;
            self.select(Showing::Builtin(Preset::AT_BOOT), reach);
            return;
        }
        #[cfg(feature = "sound3")]
        self.follow_rate(reach);
        if let Some(since) = self.dipping_since
            && (!reach.running || audio_dma::ticks_served().wrapping_sub(since) >= DIP_TICKS)
        {
            self.dipping_since = None;
            self.publish(reach);
            if self.plays_eq_now() {
                follow(reach.dsp_mode, DspMode::On);
            }
        }
    }

    pub fn toggle(&mut self, reach: &mut Reach<'_>) {
        let next = match self.showing {
            Showing::Builtin(preset) => preset.toggled(),
            Showing::Custom => Preset::Clarity,
        };
        self.select(Showing::Builtin(next), reach);
    }

    pub fn choose(&mut self, preset: Preset, reach: &mut Reach<'_>) {
        self.select(Showing::Builtin(preset), reach);
    }

    pub fn eq_on(&mut self, reach: &mut Reach<'_>) {
        self.select(self.last_eq, reach);
    }

    pub fn set_trim(&mut self, trim: PresetTrim, reach: &mut Reach<'_>) {
        self.trim = trim;
        self.change(reach);
    }

    pub fn begin(&mut self) {
        self.custom = *self.preset();
        self.staging = true;
    }

    pub fn commit(&mut self, reach: &mut Reach<'_>) -> bool {
        if !self.staging {
            return false;
        }
        self.staging = false;
        self.builtin = None;
        self.showing = Showing::Custom;
        self.last_eq = Showing::Custom;
        self.change(reach);
        true
    }

    pub fn edit_band(
        &mut self,
        index: usize,
        band: Option<dsp::Band>,
        reach: &mut Reach<'_>,
    ) -> bool {
        if index >= self.custom.bands.len() {
            return false;
        }
        if !self.staging && self.builtin.is_some() {
            self.custom = *self.preset();
        }
        let slot = &mut self.custom.bands[index];
        *slot = match band {
            Some(band) => PresetBand {
                band,
                enabled: true,
            },
            None => PresetBand {
                enabled: false,
                ..*slot
            },
        };
        if !self.staging {
            self.builtin = None;
            self.showing = Showing::Custom;
            self.last_eq = Showing::Custom;
            self.change(reach);
        }
        true
    }

    fn select(&mut self, showing: Showing, reach: &mut Reach<'_>) {
        match showing {
            Showing::Builtin(preset) => match preset.bands() {
                Some(bands) => {
                    self.builtin = Some(bands);
                    #[cfg(not(feature = "sound3"))]
                    {
                        self.trim = bands.trim;
                    }
                    #[cfg(feature = "sound3")]
                    {
                        self.trim = Self::high_rate(bands).1;
                    }
                }
                None => {
                    self.showing = showing;
                    self.dipping_since = None;
                    follow(reach.dsp_mode, DspMode::Bypass);
                    return;
                }
            },
            Showing::Custom => {}
        }
        self.showing = showing;
        self.last_eq = showing;
        self.change(reach);
    }

    fn change(&mut self, reach: &mut Reach<'_>) {
        let playing_eq = reach.dsp_mode.requested() == DspMode::On;
        if playing_eq && reach.running {
            follow(reach.dsp_mode, DspMode::Bypass);
            self.dipping_since = Some(audio_dma::ticks_served());
            return;
        }
        self.publish(reach);
        if self.plays_eq_now() {
            follow(reach.dsp_mode, DspMode::On);
        }
    }

    #[cfg(not(feature = "sound3"))]
    const fn plays_eq_now(&self) -> bool {
        self.showing.plays_eq()
    }

    #[cfg(feature = "sound3")]
    const fn plays_eq_now(&self) -> bool {
        self.showing.plays_eq() && !self.raw_above_96k
    }

    fn publish(&self, reach: &mut Reach<'_>) {
        match self.builtin {
            Some(bands) => {
                #[cfg(not(feature = "sound3"))]
                let high_rate = &bands.high_rate;
                #[cfg(feature = "sound3")]
                let high_rate = Self::high_rate(bands).0;
                reach
                    .coefficients
                    .change_presets(&bands.preset, high_rate, self.trim, reach.rate);
            }
            None => reach
                .coefficients
                .change_trimmed(&self.custom, self.trim, reach.rate),
        }
    }

    pub fn report<Out: ByteOut>(&self, reply: &mut Reply<'_, Out>, rate: SampleRate) {
        let name: &[u8] = match self.showing {
            Showing::Builtin(preset) => preset.name(),
            Showing::Custom => b"custom",
        };
        let running = bands_running_at(rate.frames_per_tick());
        #[cfg(not(feature = "sound3"))]
        let twelve = |bands: &'static Bands| &bands.high_rate;
        #[cfg(feature = "sound3")]
        let twelve = |bands: &'static Bands| Self::high_rate(bands).0;
        let enabled = match (self.builtin, rate.frames_per_tick()) {
            (Some(bands), dsp::FramesPerTick::Twelve) => twelve(bands),
            _ => self.preset(),
        }
        .bands
        .iter()
        .take(running)
        .filter(|slot| slot.enabled)
        .count();
        let path: &[u8] = match (self.showing.plays_eq(), dsp::PeqAtRate::at(rate)) {
            (false, _) => b" path=bit-perfect",
            (true, dsp::PeqAtRate::Runs) => b" path=eq",
            #[cfg(not(feature = "sound3"))]
            (true, dsp::PeqAtRate::OffAtThisRate) => b" path=eq-off-at-this-rate",
            #[cfg(feature = "sound3")]
            (true, dsp::PeqAtRate::OffAtThisRate) => b" path=bit-perfect-above-96k",
        };
        let staging: &[u8] = if self.staging { b" staging" } else { b"" };
        reply
            .text(b"preset ")
            .text(name)
            .text(b" trim=")
            .tenths(nearest_tenth(self.trim.db()))
            .field(b"bands", enabled as u32)
            .text(path)
            .text(staging)
            .end();
    }
}
