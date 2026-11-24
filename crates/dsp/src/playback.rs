use crate::biquad::Peq;
use crate::crossfade::{CrossfadeGains, OutputWords, TickMix};
use crate::limiter::{DspBlock, LimiterRamp, LimiterTrack, LookaheadPair, PeakBound};
use crate::meter::{MeterTick, StereoPeak};
use crate::monitor::{LineSource, MixedFromStream, PlaybackMix, StreamFrames};
use crate::peq::CoefficientSet;
use crate::sample::{DspSample, Frame, I2sWord, Sample, StreamSample};
use crate::tick::{FrameCountKernel, MAX_FRAMES, PlaybackBlock};

#[derive(Clone, Debug)]
pub(crate) struct PlaybackPath {
    bypass_line: OutputWords,
    lookahead: LookaheadPair,
    peq: Peq,
    limiter: LimiterTrack,
    last_mix: TickMix,
}

#[derive(Clone, Copy)]
pub(crate) enum PeqUse<'a> {
    Skipped,
    Filtering(&'a CoefficientSet),
}

#[derive(Clone, Copy)]
enum HeldPaths {
    BypassLine,
    ProcessedPair,
    Both,
}

pub(crate) struct PlaybackTick<'a, 'm, M> {
    pub(crate) path: &'a mut PlaybackPath,
    pub(crate) mix: TickMix,
    pub(crate) peq: PeqUse<'a>,
    pub(crate) mixer: M,
    pub(crate) block: &'a mut PlaybackBlock,
    pub(crate) monitored: &'a StreamFrames,
    pub(crate) meters: &'a mut MeterTick<'m>,
}

struct LineRefill<'a, S> {
    line: &'a mut OutputWords,
    source: S,
}

struct FadeTick<'a, 'm> {
    held: &'a DspBlock,
    ramp: LimiterRamp,
    gains: CrossfadeGains,
    output: &'a mut OutputWords,
    line: &'a mut OutputWords,
    held_paths: HeldPaths,
    meters: &'a mut MeterTick<'m>,
}

impl HeldPaths {
    const fn after(mix: TickMix) -> Self {
        match mix {
            TickMix::Bypass => Self::BypassLine,
            TickMix::Blend(_) => Self::Both,
            TickMix::Processed => Self::ProcessedPair,
        }
    }
}

impl PlaybackPath {
    pub(crate) const fn new(limiter: LimiterTrack) -> Self {
        Self {
            bypass_line: [[I2sWord::SILENCE; 2]; MAX_FRAMES],
            lookahead: LookaheadPair::silent(),
            peq: Peq::new(),
            limiter,
            last_mix: TickMix::Bypass,
        }
    }

    pub(crate) fn limiter(&mut self) -> &mut LimiterTrack {
        &mut self.limiter
    }

    #[cold]
    #[inline(never)]
    fn start_processing<const FRAMES: usize>(&mut self) {
        let (_, held) = self.lookahead.intake_and_held();
        for (held, line) in held.iter_mut().zip(&self.bypass_line) {
            *held = line.map(DspSample::exact_from_output);
        }
        self.peq.start_silent();
        self.limiter
            .restart_holding(PeakBound::of_block::<FRAMES>(held));
    }

    #[inline(always)]
    fn fill_intake<const FRAMES: usize, M: PlaybackMix>(
        &mut self,
        peq: PeqUse<'_>,
        input: &StreamFrames,
        monitored: &StreamFrames,
        mixer: M,
    ) -> LimiterRamp {
        let (intake, _) = self.lookahead.intake_and_held();
        let incoming_peak = match peq {
            PeqUse::Skipped => mix_into_intake::<FRAMES, M>(intake, input, monitored, mixer),
            PeqUse::Filtering(set) => {
                convert_into_intake::<FRAMES>(intake, input);
                self.peq.filter_frames::<FRAMES>(set, intake);
                mix_in_place::<FRAMES, M>(intake, monitored, mixer)
            }
        };
        self.limiter.ramp_toward::<FRAMES>(incoming_peak)
    }
}

impl<M: PlaybackMix> FrameCountKernel for PlaybackTick<'_, '_, M> {
    #[inline(never)]
    fn process<const FRAMES: usize>(self) {
        let Self {
            path,
            mix,
            peq,
            mixer,
            block,
            monitored,
            meters,
        } = self;
        let peq = peq.in_kernel::<FRAMES>();
        let held = HeldPaths::after(path.last_mix);
        path.last_mix = mix;
        let input = &block.input;
        let output = &mut block.output;
        match (mix, held) {
            (TickMix::Bypass, HeldPaths::BypassLine | HeldPaths::Both) => bypass_out::<FRAMES, _>(
                output,
                LineRefill {
                    line: &mut path.bypass_line,
                    source: MixedFromStream {
                        input,
                        monitored,
                        mixer,
                    },
                },
                meters,
            ),
            (TickMix::Bypass, HeldPaths::ProcessedPair) => {
                let ramp = path.limiter.ramp_out::<FRAMES>();
                let PlaybackPath {
                    bypass_line,
                    lookahead,
                    ..
                } = &mut *path;
                let (_, held_block) = lookahead.intake_and_held();
                limited_out_refilling_line::<FRAMES, _>(
                    held_block,
                    ramp,
                    output,
                    LineRefill {
                        line: bypass_line,
                        source: MixedFromStream {
                            input,
                            monitored,
                            mixer,
                        },
                    },
                    meters,
                );
            }
            (TickMix::Processed, held) => {
                if let HeldPaths::BypassLine = held {
                    path.start_processing::<FRAMES>();
                }
                let ramp = path.fill_intake::<FRAMES, M>(peq, input, monitored, mixer);
                let (_, held_block) = path.lookahead.intake_and_held();
                limited_out::<FRAMES>(held_block, ramp, output, meters);
                path.lookahead.advance();
            }
            (TickMix::Blend(gains), held) => {
                if let HeldPaths::BypassLine = held {
                    path.start_processing::<FRAMES>();
                }
                let ramp = path.fill_intake::<FRAMES, M>(peq, input, monitored, mixer);
                let PlaybackPath {
                    bypass_line,
                    lookahead,
                    ..
                } = &mut *path;
                let (intake, held_block) = lookahead.intake_and_held();
                let fade = FadeTick {
                    held: held_block,
                    ramp,
                    gains,
                    output,
                    line: bypass_line,
                    held_paths: held,
                    meters,
                };
                match peq {
                    PeqUse::Skipped => {
                        fade.out::<FRAMES, _>(mixer.line_when_peq_skipped(input, monitored, intake))
                    }
                    PeqUse::Filtering(_) => fade.out::<FRAMES, _>(MixedFromStream {
                        input,
                        monitored,
                        mixer,
                    }),
                }
                lookahead.advance();
            }
        }
    }
}

impl FadeTick<'_, '_> {
    #[inline(always)]
    fn out<const FRAMES: usize, S: LineSource>(self, source: S) {
        let refill = LineRefill {
            line: self.line,
            source,
        };
        match self.held_paths {
            HeldPaths::ProcessedPair => limited_out_refilling_line::<FRAMES, S>(
                self.held,
                self.ramp,
                self.output,
                refill,
                self.meters,
            ),
            HeldPaths::BypassLine | HeldPaths::Both => blended_out::<FRAMES, S>(
                self.held,
                self.ramp,
                self.gains,
                self.output,
                refill,
                self.meters,
            ),
        }
    }
}

#[inline(always)]
fn converted(frame: Frame<StreamSample>) -> Frame<DspSample> {
    frame.map(DspSample::exact_from_stream)
}

#[inline(always)]
fn limited(frame: Frame<DspSample>, ramp: &mut LimiterRamp) -> Frame<I2sWord> {
    let gain = ramp.next_frame();
    frame.map(|sample| gain.apply_onto_output(sample))
}

#[inline(never)]
fn mix_into_intake<const FRAMES: usize, M: PlaybackMix>(
    intake: &mut DspBlock,
    input: &StreamFrames,
    monitored: &StreamFrames,
    mixer: M,
) -> PeakBound {
    let mut peak = StereoPeak::SILENCE;
    let lanes = intake.iter_mut().zip(input).zip(monitored);
    for ((slot, input), monitored) in lanes.take(FRAMES) {
        let frame = mixer.mixed(converted(*input), *monitored);
        peak = peak.observe(frame);
        *slot = frame;
    }
    PeakBound::of_stereo_peak(peak)
}

#[inline(never)]
fn convert_into_intake<const FRAMES: usize>(intake: &mut DspBlock, input: &StreamFrames) {
    for (slot, input) in intake.iter_mut().zip(input).take(FRAMES) {
        *slot = converted(*input);
    }
}

#[inline(never)]
fn mix_in_place<const FRAMES: usize, M: PlaybackMix>(
    intake: &mut DspBlock,
    monitored: &StreamFrames,
    mixer: M,
) -> PeakBound {
    let mut peak = StereoPeak::SILENCE;
    for (slot, monitored) in intake.iter_mut().zip(monitored).take(FRAMES) {
        let frame = mixer.mixed(*slot, *monitored);
        peak = peak.observe(frame);
        *slot = frame;
    }
    PeakBound::of_stereo_peak(peak)
}

#[inline(never)]
fn bypass_out<const FRAMES: usize, S: LineSource>(
    output: &mut OutputWords,
    refill: LineRefill<'_, S>,
    meters: &mut MeterTick<'_>,
) {
    let mut peak = StereoPeak::SILENCE;
    let lanes = output
        .iter_mut()
        .zip(refill.line.iter_mut())
        .zip(refill.source.words());
    for ((output, line), incoming) in lanes.take(FRAMES) {
        let words = *line;
        *line = incoming;
        peak = peak.observe(words);
        *output = words;
    }
    meters.raise_playback(peak);
}

#[inline(never)]
fn limited_out<const FRAMES: usize>(
    held: &DspBlock,
    mut ramp: LimiterRamp,
    output: &mut OutputWords,
    meters: &mut MeterTick<'_>,
) {
    #[cfg(feature = "meter-limiting")]
    let limiting = ramp.limiting();
    let mut peak = StereoPeak::SILENCE;
    for (output, held) in output.iter_mut().zip(held).take(FRAMES) {
        let words = limited(*held, &mut ramp);
        peak = peak.observe(words);
        *output = words;
    }
    meters.raise_playback(peak);
    #[cfg(feature = "meter-limiting")]
    meters.count_limiting(limiting);
}

#[inline(never)]
fn limited_out_refilling_line<const FRAMES: usize, S: LineSource>(
    held: &DspBlock,
    mut ramp: LimiterRamp,
    output: &mut OutputWords,
    refill: LineRefill<'_, S>,
    meters: &mut MeterTick<'_>,
) {
    #[cfg(feature = "meter-limiting")]
    let limiting = ramp.limiting();
    let mut peak = StereoPeak::SILENCE;
    let lanes = output
        .iter_mut()
        .zip(held)
        .zip(refill.line.iter_mut())
        .zip(refill.source.words());
    for (((output, held), line), incoming) in lanes.take(FRAMES) {
        let words = limited(*held, &mut ramp);
        *line = incoming;
        peak = peak.observe(words);
        *output = words;
    }
    meters.raise_playback(peak);
    #[cfg(feature = "meter-limiting")]
    meters.count_limiting(limiting);
}

#[inline(never)]
fn blended_out<const FRAMES: usize, S: LineSource>(
    held: &DspBlock,
    mut ramp: LimiterRamp,
    gains: CrossfadeGains,
    output: &mut OutputWords,
    refill: LineRefill<'_, S>,
    meters: &mut MeterTick<'_>,
) {
    #[cfg(feature = "meter-limiting")]
    let limiting = ramp.limiting();
    let mut peak = StereoPeak::SILENCE;
    let lanes = output
        .iter_mut()
        .zip(held)
        .zip(refill.line.iter_mut())
        .enumerate();
    for (frame, ((output, held), line)) in lanes.take(FRAMES) {
        let processed = limited(*held, &mut ramp);
        let bypass = *line;
        *line = refill.source.words_at(frame);
        let words = [
            gains.mix(bypass[0], processed[0]),
            gains.mix(bypass[1], processed[1]),
        ];
        peak = peak.observe(words);
        *output = words;
    }
    meters.raise_playback(peak);
    #[cfg(feature = "meter-limiting")]
    meters.count_limiting(limiting);
}
