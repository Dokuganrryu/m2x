mod boot_board;

use boot_board::{Bench, Seen, run_bench};
use device::analog_settings::AnalogSettings;
use device::boot::{State, VolumeTargets};
use device::command::Command;
use device::control_pass::{ControlPass, PanelReadings, PassOutputs};
use device::dac::{DacWrite, VolumeRegister};
use device::expander::ExpanderInputs;
use device::knob::{Knob, KnobReading, KnobReadings, Knobs};
use device::knob_law::volume_targets;
use device::phantom::{Jack, PhantomGrant};

const SEQUENCES: u64 = 10_000;
const PASSES: usize = 1_000;
const PASSES_TO_SETTLE_ANY_SLEW: usize = 256;
const NOISE_WINDOW_NARROWER_THAN_THE_HYSTERESIS_BAND: u16 = 0x7E;
const NO_BUTTON_PRESSED: ExpanderInputs = ExpanderInputs(0xFF);

struct XorShift(u64);

impl XorShift {
    fn seeded(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn readings(raw: [u16; 5]) -> PanelReadings {
    PanelReadings {
        knobs: KnobReadings(raw.map(KnobReading)),
        jacks: [Jack::Removed; 2],
        buttons: NO_BUTTON_PRESSED,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Levels {
    main: u8,
    headphone: u8,
}

impl Levels {
    fn of(targets: VolumeTargets) -> Self {
        Self {
            main: targets.main.half_decibel_steps(),
            headphone: targets.headphone.half_decibel_steps(),
        }
    }

    fn after(mut self, outputs: &PassOutputs, context: impl std::fmt::Debug + Copy) -> Self {
        let mut written: Vec<(VolumeRegister, u8)> = Vec::new();
        for command in outputs.commands.as_slice() {
            if let Command::WriteDac(DacWrite::Volume(register, level)) = command {
                written.push((*register, level.half_decibel_steps()));
            }
        }
        for pair in written.chunks(2) {
            match pair {
                [(VolumeRegister::MainA, a), (VolumeRegister::MainB, b)] if a == b => {
                    assert_ne!(*a, self.main, "{context:?} rewrote an unchanged main level");
                    self.main = *a;
                }
                [
                    (VolumeRegister::HeadphoneA, a),
                    (VolumeRegister::HeadphoneB, b),
                ] if a == b => {
                    assert_ne!(*a, self.headphone, "{context:?} rewrote an unchanged level");
                    self.headphone = *a;
                }
                other => panic!("{context:?} wrote an unpaired volume {other:?}"),
            }
        }
        self
    }
}

fn check_path(before: u8, after: u8, target: u8, path: &str, context: impl std::fmt::Debug + Copy) {
    assert!(
        after >= target,
        "{context:?} {path} at {after} is louder than the knob asks ({target})"
    );
    assert!(
        after >= before.saturating_sub(1),
        "{context:?} {path} got louder by more than one half decibel ({before} to {after})"
    );
    let toward = (before.min(target)..=before.max(target)).contains(&after);
    assert!(
        toward,
        "{context:?} {path} overshot ({before} to {after}, target {target})"
    );
}

fn knob_trace(seed: u64) {
    let mut random = XorShift::seeded(seed);
    let mut rest = [0u16; 5].map(|_| random.next() as u16);
    let first = readings(rest);
    let mut model = Knobs::first_scan(first.knobs);
    let ramp_end = volume_targets(model);
    let mut pass = ControlPass::after_boot(model, ramp_end);
    let mut analog = AnalogSettings::AT_BOOT;
    let mut levels = Levels::of(ramp_end);
    let mut target_since = 0usize;
    let mut last_target = Levels::of(ramp_end);
    let mut at_rest: Vec<(u8, u8)> = Vec::new();
    for index in 0..PASSES {
        for raw in rest.iter_mut() {
            match random.below(100) {
                0 => *raw = [0, u16::MAX][random.below(2) as usize],
                1 | 2 => *raw = random.next() as u16,
                3..=9 => *raw = raw.wrapping_add((random.below(0x600) as u16).wrapping_sub(0x300)),
                _ => {}
            }
        }
        let noisy = rest.map(|raw| {
            raw.saturating_add(random.below(0x40) as u16)
                .saturating_sub(0x20)
        });
        let scan = readings(noisy);
        model = model.after_scan(scan.knobs);
        let target = Levels::of(volume_targets(model));
        let context = ("seed", seed, "pass", index);
        let (next, outputs) = pass.run(scan, analog, PhantomGrant::NONE);
        let after = levels.after(&outputs, context);
        check_path(levels.main, after.main, target.main, "main", context);
        check_path(
            levels.headphone,
            after.headphone,
            target.headphone,
            "headphone",
            context,
        );
        if target != last_target {
            last_target = target;
            target_since = index;
        }
        if index - target_since >= PASSES_TO_SETTLE_ANY_SLEW {
            assert_eq!(
                after, target,
                "{context:?} did not settle on its target at rest"
            );
            at_rest.push((model.position(Knob::Main).value(), after.main));
            at_rest.push((model.position(Knob::Headphone).value(), after.headphone));
        }
        levels = after;
        analog = outputs.analog;
        pass = next;
    }
    at_rest.sort_unstable();
    assert!(
        at_rest.windows(2).all(|pair| pair[0].1 <= pair[1].1),
        "seed {seed} commanded level is not monotonic in knob position at rest"
    );
}

#[test]
fn over_random_knob_traces_no_output_is_louder_than_the_knob_asks_or_overshoots_and_rest_is_monotonic()
 {
    for seed in 0..SEQUENCES {
        knob_trace(seed);
    }
}

fn noise_at_rest(seed: u64) {
    let mut random = XorShift::seeded(seed);
    let top = [0u16; 5]
        .map(|_| (random.next() as u16).max(NOISE_WINDOW_NARROWER_THAN_THE_HYSTERESIS_BAND));
    let knobs = Knobs::first_scan(readings(top).knobs);
    let quietest_ramp_end = volume_targets(Knobs::first_scan(readings([u16::MAX; 5]).knobs));
    let mut pass = ControlPass::after_boot(knobs, quietest_ramp_end);
    let mut analog = AnalogSettings::AT_BOOT;
    let mut settled = None;
    for _ in 0..PASSES_TO_SETTLE_ANY_SLEW {
        let (next, outputs) = pass.run(readings(top), analog, PhantomGrant::NONE);
        pass = next;
        analog = outputs.analog;
        settled = Some(outputs.tick);
    }
    for index in 0..PASSES {
        let noisy = top.map(|raw| {
            raw - random.below(u64::from(NOISE_WINDOW_NARROWER_THAN_THE_HYSTERESIS_BAND) + 1) as u16
        });
        let (next, outputs) = pass.run(readings(noisy), analog, PhantomGrant::NONE);
        assert_eq!(
            outputs.commands.as_slice(),
            [],
            "seed {seed} pass {index} emitted a command at rest"
        );
        assert_eq!(Some(outputs.tick), settled, "seed {seed} pass {index}");
        assert_eq!(outputs.analog, analog);
        pass = next;
    }
}

#[test]
fn noise_inside_the_hysteresis_band_at_rest_emits_no_command_and_moves_no_gain() {
    for seed in 0..SEQUENCES / 10 {
        noise_at_rest(seed);
    }
}

fn last_written(trace: &[Seen], register: VolumeRegister) -> u8 {
    trace
        .iter()
        .rev()
        .find_map(|seen| match seen {
            Seen::Command(Command::WriteDac(DacWrite::Volume(r, level))) if *r == register => {
                Some(level.half_decibel_steps())
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn the_slew_takes_over_from_the_boot_ramp_with_no_jump() {
    let raw = [0x7800, 0x6400, 0x7800, 0x7800, 0x9600];
    let knobs = Knobs::first_scan(readings(raw).knobs);
    let bench = Bench {
        targets: volume_targets(knobs),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, 2_000);
    assert!(matches!(caller.boot, State::Running(..)));
    let ramp_end = Levels {
        main: last_written(&caller.trace, VolumeRegister::MainB),
        headphone: last_written(&caller.trace, VolumeRegister::HeadphoneB),
    };
    assert_eq!(ramp_end, Levels::of(bench.targets));

    let pass = ControlPass::after_boot(knobs, bench.targets);
    let (_, unchanged) = pass.run(readings(raw), AnalogSettings::AT_BOOT, PhantomGrant::NONE);
    assert_eq!(ramp_end.after(&unchanged, "unchanged knobs"), ramp_end);

    let mut louder = raw;
    louder[1] = 0;
    louder[4] = 0;
    let (_, turned) = pass.run(
        readings(louder),
        AnalogSettings::AT_BOOT,
        PhantomGrant::NONE,
    );
    assert_eq!(
        ramp_end.after(&turned, "louder knobs"),
        Levels {
            main: ramp_end.main - 1,
            headphone: ramp_end.headphone - 1,
        }
    );
}
