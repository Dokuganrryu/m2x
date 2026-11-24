use device::boot::BOOT_RATE;
use dsp::{
    CaptureBlock, CoefficientTick, Crossfade, DspModeSwitch, FramesPerTick, Gain, InputMute,
    LimiterCoefficients, LimiterFollower, LimiterSettings, MeterTick, MonitorMix, Pipeline,
    PlaybackBlock, PublishesSeen, RateFollower, SampleRate, TickControls,
};
use stream::{PlaybackTick, RingWriter, TickBlocks};

use super::audio_dma::{self, AUDIO};
use super::handovers::{CAPTURE_GAINS, INPUT_MUTES, LIMITER, MONITOR, PUBLISHES, RATE};
use super::tick_timer;

const BOOT_LIMITER: LimiterCoefficients =
    LimiterCoefficients::design(LimiterSettings::DEFAULT, BOOT_RATE.family());

pub struct TickContext {
    pipeline: Pipeline,
    crossfade: Crossfade,
    frames: FramesPerTick,
    publishes: PublishesSeen,
    capture_gains: [Gain; 2],
    monitor: MonitorMix,
    inputs: [InputMute; 2],
    rate: RateFollower,
    limiter: LimiterCoefficients,
    limiter_follower: LimiterFollower,
    playback: PlaybackTick<'static>,
    capture: RingWriter<'static>,
    coefficients: CoefficientTick<'static>,
    meters: MeterTick<'static>,
    dsp_mode: &'static DspModeSwitch,
    playback_block: PlaybackBlock,
    capture_block: CaptureBlock,
}

impl TickContext {
    pub fn new(
        playback: PlaybackTick<'static>,
        capture: RingWriter<'static>,
        coefficients: CoefficientTick<'static>,
        meters: MeterTick<'static>,
        dsp_mode: &'static DspModeSwitch,
    ) -> Self {
        let limiter = BOOT_LIMITER;
        let publishes = PublishesSeen::of(&PUBLISHES);
        Self {
            pipeline: Pipeline::new(limiter),
            crossfade: Crossfade::new(),
            frames: BOOT_RATE.frames_per_tick(),
            publishes,
            capture_gains: CAPTURE_GAINS.latest(),
            monitor: MONITOR.latest_or(MonitorMix::OFF),
            inputs: INPUT_MUTES.latest(),
            rate: RateFollower::of(&RATE),
            limiter,
            limiter_follower: LimiterFollower::of(&LIMITER),
            playback,
            capture,
            coefficients,
            meters,
            dsp_mode,
            playback_block: PlaybackBlock::new(),
            capture_block: CaptureBlock::new(),
        }
    }

    pub fn run(&mut self) {
        tick_timer::acknowledge();
        #[cfg(image_plant = "division_in_tick")]
        super::image_plants::divide_in_tick();
        let reading = audio_dma::receive_reading();
        if self.publishes.any_new(&PUBLISHES) {
            self.adopt_what_thread_mode_published();
        }
        let frames = self.frames;
        let blocks = TickBlocks::receive(reading, frames, &AUDIO, &mut self.capture_block.input);
        let mix = self.crossfade.tick(self.dsp_mode);
        self.playback.render(frames, &mut self.playback_block.input);
        let controls = TickControls {
            frames,
            mix,
            coefficients: self.coefficients.adopt(),
            monitor: self.monitor,
            inputs: self.inputs,
            capture_gains: self.capture_gains,
        };
        self.pipeline.tick(
            &controls,
            &mut self.playback_block,
            &mut self.capture_block,
            &mut self.meters,
        );
        blocks.transmit(&self.playback_block.output, &AUDIO);
        let captured = self.capture_block.output.split_at(frames.count()).0;
        self.capture.push(captured);
    }

    #[cold]
    #[inline(never)]
    fn adopt_what_thread_mode_published(&mut self) {
        self.capture_gains = CAPTURE_GAINS.latest();
        self.monitor = MONITOR.latest_or(self.monitor);
        self.inputs = INPUT_MUTES.latest();
        if let Some(limiter) = self.limiter_follower.changed(&LIMITER) {
            self.limiter = limiter;
            self.pipeline.retune_limiter(limiter);
        }
        if let Some(rate) = self.rate.changed(&RATE) {
            self.restart_at(rate);
        }
    }

    fn restart_at(&mut self, rate: SampleRate) {
        self.frames = rate.frames_per_tick();
        self.pipeline = Pipeline::new(self.limiter);
    }
}
