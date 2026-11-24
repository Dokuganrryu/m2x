use dsp::{
    CaptureBlock, CoefficientSet, Crossfade, DspModeSwitch, Frame, Gain, Generation, I2sWord,
    InputMute, LimiterCoefficients, LimiterSettings, Meters, MonitorMix, Pipeline, PlaybackBlock,
    SampleRate, StreamSample, TickControls, TickMix,
};

use crate::digest::TickDigest;
use crate::grid::five_band_preset;
use crate::script::{Cue, STANDARD_SCRIPT, Script};
use crate::source::{FrameSource, SeededNoise};

pub struct GoldenRun<S> {
    rate: SampleRate,
    script: Script,
    source: S,
    tick: u16,
    next_cue: usize,
    pipeline: Pipeline,
    switch: DspModeSwitch,
    crossfade: Crossfade,
    coefficients: CoefficientSet,
    monitor: MonitorMix,
    capture_gains: [Gain; 2],
    meters: Meters,
    playback: PlaybackBlock,
    capture: CaptureBlock,
}

pub struct TickWords<'a> {
    pub tick: u16,
    pub mix: TickMix,
    pub playback: &'a [Frame<I2sWord>],
    pub capture: &'a [Frame<StreamSample>],
}

impl TickWords<'_> {
    pub fn digest(&self) -> TickDigest {
        TickDigest::of_tick(self.playback, self.capture)
    }
}

pub fn standard_run(rate: SampleRate) -> GoldenRun<SeededNoise> {
    GoldenRun::new(rate, STANDARD_SCRIPT, SeededNoise::seeded(rate.hz()))
}

impl<S: FrameSource> GoldenRun<S> {
    pub fn new(rate: SampleRate, script: Script, source: S) -> Self {
        Self {
            rate,
            script,
            source,
            tick: 0,
            next_cue: 0,
            pipeline: Pipeline::new(LimiterCoefficients::design(
                LimiterSettings::default(),
                rate.family(),
            )),
            switch: DspModeSwitch::new(),
            crossfade: Crossfade::new(),
            coefficients: CoefficientSet::design(&five_band_preset(), rate, Generation::FIRST),
            monitor: MonitorMix::OFF,
            capture_gains: [Gain::UNITY; 2],
            meters: Meters::new(),
            playback: PlaybackBlock::new(),
            capture: CaptureBlock::new(),
        }
    }

    fn apply_cues_due(&mut self) {
        while let Some(due) = self
            .script
            .cues
            .get(self.next_cue)
            .filter(|cue| cue.tick == self.tick)
        {
            match due.cue {
                Cue::Dsp(mode) => self.switch.request(mode),
                Cue::CaptureGains(gains) => self.capture_gains = gains,
                Cue::Monitor(mix) => self.monitor = mix,
            }
            self.next_cue += 1;
        }
    }

    pub fn advance(&mut self) -> Option<TickWords<'_>> {
        if self.tick >= self.script.ticks {
            return None;
        }
        self.apply_cues_due();
        let frames = self.rate.frames_per_tick();
        let n = frames.count();
        for (playback, adc) in self.playback.input[..n]
            .iter_mut()
            .zip(&mut self.capture.input[..n])
        {
            let frame = self.source.next_frame();
            *playback = frame.playback;
            *adc = frame.adc;
        }
        let mix = self.crossfade.tick(&self.switch);
        let controls = TickControls {
            frames,
            mix,
            coefficients: &self.coefficients,
            monitor: self.monitor,
            inputs: [InputMute::Live; 2],
            capture_gains: self.capture_gains,
        };
        let (mut meter_tick, _) = self.meters.split();
        self.pipeline.tick(
            &controls,
            &mut self.playback,
            &mut self.capture,
            &mut meter_tick,
        );
        let tick = self.tick;
        self.tick += 1;
        Some(TickWords {
            tick,
            mix,
            playback: &self.playback.output[..n],
            capture: &self.capture.output[..n],
        })
    }
}
