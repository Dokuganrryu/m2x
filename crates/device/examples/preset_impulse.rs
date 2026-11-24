use std::fs;
use std::path::PathBuf;

use device::preset::Preset;
use dsp::{
    CaptureBlock, CoefficientSet, FramesPerTick, Gain, Generation, I2sWord, InputMute, Meters,
    MonitorMix, Pipeline, PlaybackBlock, SampleRate, StreamSample, TickControls, TickMix,
};

const USAGE: &str = "usage: preset_impulse <out folder>; writes <preset>_<rate>.f64, the left-channel impulse response at -12 dBFS through dsp::Pipeline with the preset's trimmed coefficient set, 65,536 samples, full scale 1.0";
const RESPONSE_SAMPLES: usize = 1 << 16;
const IMPULSE_CODE: i32 = 1 << 21;
const STREAM_SHIFT: u32 = 8;
const FULL_SCALE: f64 = (1u64 << 31) as f64;
const WARM_UP_TICKS: usize = 64;

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect(USAGE));
    fs::create_dir_all(&out).expect("the output folder is writable");
    #[cfg(feature = "sound3")]
    dsp::lift_bands_at_twelve_frames();
    for preset in [Preset::Clarity, Preset::Warm] {
        let bands = preset.bands().expect("a built-in EQ preset");
        #[cfg(not(feature = "sound3"))]
        let (high_rate, trim) = (&bands.high_rate, bands.trim);
        #[cfg(feature = "sound3")]
        let (high_rate, trim) =
            bands.for_twelve_frame_bands(dsp::bands_running_at(FramesPerTick::Twelve));
        for rate in [
            SampleRate::Hz44100,
            SampleRate::Hz48000,
            SampleRate::Hz96000,
        ] {
            let chosen = match rate.frames_per_tick() {
                FramesPerTick::Twelve => high_rate,
                FramesPerTick::Six | FramesPerTick::TwentyFour => &bands.preset,
            };
            let set = CoefficientSet::design_trimmed(chosen, trim, rate, Generation::FIRST);
            let response = impulse_response(&set, rate);
            let bytes: Vec<u8> = response.iter().flat_map(|v| v.to_le_bytes()).collect();
            let name = format!(
                "{}_{}.f64",
                String::from_utf8_lossy(preset.name()),
                rate.hz()
            );
            fs::write(out.join(&name), bytes).expect("the response is written");
            println!(
                "{name}: {} bands enabled, trim {:.1} dB",
                set.enabled_mask().count_ones(),
                trim.db()
            );
        }
    }
}

fn impulse_response(set: &CoefficientSet, rate: SampleRate) -> Vec<f64> {
    let frames = rate.frames_per_tick();
    let count = frames.count();
    let mut pipeline = Pipeline::default();
    let mut meters = Meters::new();
    let (mut meter_tick, _) = meters.split();
    let mut capture = CaptureBlock::new();
    let mut block = PlaybackBlock::new();
    let controls = TickControls {
        frames,
        mix: TickMix::Processed,
        coefficients: set,
        monitor: MonitorMix::OFF,
        inputs: [InputMute::Live; 2],
        capture_gains: [Gain::UNITY; 2],
    };
    let mut output = Vec::with_capacity(RESPONSE_SAMPLES + WARM_UP_TICKS * count);
    let mut fed = 0usize;
    while output.len() < RESPONSE_SAMPLES + WARM_UP_TICKS * count {
        for frame in block.input[..count].iter_mut() {
            let code = if fed == WARM_UP_TICKS * count {
                IMPULSE_CODE
            } else {
                0
            };
            *frame = [StreamSample::from_bits(code << STREAM_SHIFT); 2];
            fed += 1;
        }
        pipeline.tick(&controls, &mut block, &mut capture, &mut meter_tick);
        output.extend(
            block.output[..count]
                .iter()
                .map(|frame| f64::from(I2sWord::to_bits(frame[0])) / FULL_SCALE),
        );
    }
    let impulse = f64::from(IMPULSE_CODE << STREAM_SHIFT) / FULL_SCALE;
    output[WARM_UP_TICKS * count..]
        .iter()
        .take(RESPONSE_SAMPLES)
        .map(|value| value / impulse)
        .collect()
}
