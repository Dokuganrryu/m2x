use device::analog_settings::AnalogSettings;
use device::analog_word::PreampCode;
use device::boot::VolumeTargets;
use device::command::Command;
use device::control_pass::{ControlPass, Monitoring, PanelReadings, PassOutputs};
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::expander::ExpanderInputs;
use device::knob::{Knob, KnobPosition, KnobReading, KnobReadings, Knobs};
use device::knob_law::{monitor_blend, preamp_gain, volume_attenuation, volume_targets};
use device::phantom::{Jack, PhantomGrant};
use dsp::{Gain, MonitorMix};

const NO_BUTTON_PRESSED: ExpanderInputs = ExpanderInputs(0xFF);
const MONITOR_ONE_PRESSED: ExpanderInputs = ExpanderInputs(!(1 << 7));
const MONITOR_TWO_PRESSED: ExpanderInputs = ExpanderInputs(!(1 << 2));

#[derive(Clone, Copy)]
struct Panel {
    positions: [u8; 5],
    jacks: [Jack; 2],
    buttons: ExpanderInputs,
}

impl Panel {
    const RESTING: Self = Self {
        positions: [120, 100, 120, 120, 150],
        jacks: [Jack::Removed; 2],
        buttons: NO_BUTTON_PRESSED,
    };

    fn with(mut self, knob: Knob, position: u8) -> Self {
        self.positions[knob_index(knob)] = position;
        self
    }

    fn readings(self) -> PanelReadings {
        PanelReadings {
            knobs: KnobReadings(self.positions.map(|k| KnobReading(u16::from(k) << 8))),
            jacks: self.jacks,
            buttons: self.buttons,
        }
    }

    fn knobs(self) -> Knobs {
        Knobs::first_scan(self.readings().knobs)
    }
}

fn knob_index(knob: Knob) -> usize {
    Knob::ALL.iter().position(|k| *k == knob).unwrap()
}

fn at(k: u8) -> KnobPosition {
    KnobPosition::new(k).unwrap()
}

struct Rig {
    pass: ControlPass,
    analog: AnalogSettings,
}

impl Rig {
    fn booted(panel: Panel) -> Self {
        let knobs = panel.knobs();
        Self {
            pass: ControlPass::after_boot(knobs, volume_targets(knobs)),
            analog: AnalogSettings::AT_BOOT,
        }
    }

    fn run(&mut self, panel: Panel) -> PassOutputs {
        let (pass, outputs) = self
            .pass
            .run(panel.readings(), self.analog, PhantomGrant::NONE);
        self.pass = pass;
        self.analog = outputs.analog;
        outputs
    }
}

fn volume_writes(outputs: &PassOutputs) -> Vec<(VolumeRegister, u8)> {
    outputs
        .commands
        .as_slice()
        .iter()
        .filter_map(|command| match command {
            Command::WriteDac(DacWrite::Volume(register, level)) => {
                Some((*register, level.half_decibel_steps()))
            }
            _ => None,
        })
        .collect()
}

fn steps(k: u8) -> u8 {
    volume_attenuation(at(k)).half_decibel_steps()
}

#[test]
fn the_first_pass_after_the_boot_ramp_writes_no_volume_and_sends_the_knob_preamp_codes() {
    let mut rig = Rig::booted(Panel::RESTING);
    let outputs = rig.run(Panel::RESTING);
    let one = preamp_gain(at(120), Jack::Removed).code;
    let two = preamp_gain(at(120), Jack::Removed).code;
    let expected = AnalogSettings {
        preamp_one: one,
        preamp_two: two,
        ..AnalogSettings::AT_BOOT
    };
    assert_eq!(
        outputs.commands.as_slice(),
        [Command::SendAnalogWord(expected.word(PhantomGrant::NONE))]
    );
    assert_eq!(outputs.analog, expected);
}

#[test]
fn a_pass_with_nothing_changed_emits_no_command() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    assert_eq!(rig.run(Panel::RESTING).commands.as_slice(), []);
}

#[test]
fn turning_main_louder_steps_both_main_registers_one_half_decibel_per_pass() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    let louder = Panel::RESTING.with(Knob::Main, 90);
    let start = steps(100);
    let end = steps(90);
    for level in (end..start).rev() {
        assert_eq!(
            volume_writes(&rig.run(louder)),
            [
                (VolumeRegister::MainA, level),
                (VolumeRegister::MainB, level)
            ]
        );
    }
    assert_eq!(volume_writes(&rig.run(louder)), []);
}

#[test]
fn turning_main_quieter_lands_on_the_target_in_one_pass() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    let quieter = Panel::RESTING.with(Knob::Main, 240);
    assert_eq!(
        volume_writes(&rig.run(quieter)),
        [(VolumeRegister::MainA, 255), (VolumeRegister::MainB, 255)]
    );
    assert_eq!(volume_writes(&rig.run(quieter)), []);
}

#[test]
fn the_headphone_knob_moves_only_the_headphone_registers() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    let louder = Panel::RESTING.with(Knob::Headphone, 149);
    assert_eq!(
        volume_writes(&rig.run(louder)),
        [
            (VolumeRegister::HeadphoneA, steps(150) - 1),
            (VolumeRegister::HeadphoneB, steps(150) - 1)
        ]
    );
}

#[test]
fn the_analog_code_and_the_digital_gain_change_in_the_same_pass() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    let turned = Panel::RESTING.with(Knob::PreampOne, 40);
    let outputs = rig.run(turned);
    let gain = preamp_gain(at(40), Jack::Removed);
    assert_eq!(outputs.analog.preamp_one, gain.code);
    assert_eq!(
        outputs.commands.as_slice(),
        [Command::SendAnalogWord(
            outputs.analog.word(PhantomGrant::NONE)
        )]
    );
    assert_eq!(outputs.tick.capture[0], gain.digital);
    assert_eq!(
        outputs.tick.capture[1],
        preamp_gain(at(120), Jack::Removed).digital
    );
}

#[test]
fn a_jack_on_input_two_raises_only_preamp_two_by_one_code() {
    let mut rig = Rig::booted(Panel::RESTING);
    let removed = rig.run(Panel::RESTING);
    let inserted = rig.run(Panel {
        jacks: [Jack::Removed, Jack::Inserted],
        ..Panel::RESTING
    });
    assert_eq!(
        inserted.analog.preamp_two.value(),
        removed.analog.preamp_two.value() + 1
    );
    assert_eq!(inserted.analog.preamp_one, removed.analog.preamp_one);
}

#[test]
fn a_digital_only_preamp_change_reaches_the_tick_with_no_analog_word() {
    let panel = Panel::RESTING.with(Knob::PreampOne, 10);
    let mut rig = Rig::booted(panel);
    rig.run(panel);
    let outputs = rig.run(panel.with(Knob::PreampOne, 0));
    assert_eq!(outputs.analog.preamp_one, PreampCode::MAXIMUM);
    assert_eq!(outputs.commands.as_slice(), []);
    assert_eq!(
        outputs.tick.capture[0],
        preamp_gain(at(0), Jack::Removed).digital
    );
}

#[test]
fn monitoring_is_off_at_boot_so_playback_passes_at_unity_and_no_input_is_heard() {
    let mut rig = Rig::booted(Panel::RESTING);
    assert_eq!(rig.run(Panel::RESTING).tick.monitor, MonitorMix::OFF);
}

fn press(rig: &mut Rig, pressed: ExpanderInputs, passes: usize) -> PassOutputs {
    for _ in 0..passes {
        rig.run(Panel {
            buttons: pressed,
            ..Panel::RESTING
        });
    }
    rig.run(Panel::RESTING)
}

#[test]
fn a_short_press_of_expander_bit_7_monitors_input_one_through_the_mix_law() {
    let mut rig = Rig::booted(Panel::RESTING);
    let outputs = press(&mut rig, MONITOR_ONE_PRESSED, 10);
    let blend = monitor_blend(at(120));
    assert_eq!(
        outputs.tick.monitor,
        MonitorMix {
            playback: blend.playback,
            inputs: [blend.inputs, Gain::ZERO],
        }
    );
}

#[test]
fn a_short_press_of_expander_bit_2_monitors_input_two_and_a_second_press_stops_it() {
    let mut rig = Rig::booted(Panel::RESTING);
    let blend = monitor_blend(at(120));
    let on = press(&mut rig, MONITOR_TWO_PRESSED, 10);
    assert_eq!(on.tick.monitor.inputs, [Gain::ZERO, blend.inputs]);
    let off = press(&mut rig, MONITOR_TWO_PRESSED, 10);
    assert_eq!(off.tick.monitor, MonitorMix::OFF);
}

#[test]
fn the_pass_reports_which_inputs_are_monitored_for_the_panel_leds() {
    let mut rig = Rig::booted(Panel::RESTING);
    assert_eq!(rig.pass.monitoring(), [Monitoring::Off; 2]);
    press(&mut rig, MONITOR_ONE_PRESSED, 10);
    assert_eq!(rig.pass.monitoring(), [Monitoring::On, Monitoring::Off]);
    press(&mut rig, MONITOR_TWO_PRESSED, 10);
    assert_eq!(rig.pass.monitoring(), [Monitoring::On, Monitoring::On]);
}

#[test]
fn a_press_shorter_than_30_ms_or_longer_than_1_s_does_not_toggle_monitoring() {
    let mut rig = Rig::booted(Panel::RESTING);
    assert_eq!(
        press(&mut rig, MONITOR_ONE_PRESSED, 5).tick.monitor,
        MonitorMix::OFF
    );
    assert_eq!(
        press(&mut rig, MONITOR_ONE_PRESSED, 201).tick.monitor,
        MonitorMix::OFF
    );
    assert_ne!(
        press(&mut rig, MONITOR_ONE_PRESSED, 6).tick.monitor,
        MonitorMix::OFF
    );
}

#[test]
fn the_mix_knob_moves_the_monitored_blend() {
    let mut rig = Rig::booted(Panel::RESTING);
    press(&mut rig, MONITOR_ONE_PRESSED, 10);
    let outputs = rig.run(Panel::RESTING.with(Knob::MonitorMix, 0));
    assert_eq!(
        outputs.tick.monitor,
        MonitorMix {
            playback: Gain::ZERO,
            inputs: [Gain::UNITY, Gain::ZERO],
        }
    );
}

#[test]
fn the_boot_targets_are_the_trim_law_at_the_main_and_headphone_knobs() {
    assert_eq!(
        volume_targets(Panel::RESTING.knobs()),
        VolumeTargets {
            main: Attenuation::from_half_decibel_steps(steps(100)),
            headphone: Attenuation::from_half_decibel_steps(steps(150)),
        }
    );
}

#[test]
fn after_a_rate_change_the_volume_ramps_up_from_full_attenuation_one_step_per_pass() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    rig.pass = rig.pass.with_volume_from_full_attenuation();
    let full = Attenuation::FULL.half_decibel_steps();
    let main = steps(100).max(full - 1);
    let headphone = steps(150).max(full - 1);
    assert_eq!(
        volume_writes(&rig.run(Panel::RESTING)),
        [
            (VolumeRegister::MainA, main),
            (VolumeRegister::MainB, main),
            (VolumeRegister::HeadphoneA, headphone),
            (VolumeRegister::HeadphoneB, headphone),
        ]
    );
    assert_eq!(
        volume_writes(&rig.run(Panel::RESTING)),
        [
            (VolumeRegister::MainA, full - 2),
            (VolumeRegister::MainB, full - 2),
            (VolumeRegister::HeadphoneA, full - 2),
            (VolumeRegister::HeadphoneB, full - 2),
        ]
    );
}

#[test]
fn the_ramp_from_full_attenuation_keeps_the_monitor_toggles_and_the_knob_cache() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel {
        buttons: MONITOR_ONE_PRESSED,
        ..Panel::RESTING
    });
    for _ in 0..20 {
        rig.run(Panel {
            buttons: MONITOR_ONE_PRESSED,
            ..Panel::RESTING
        });
    }
    let monitored = rig.run(Panel::RESTING).tick.monitor;
    assert!(monitored.monitors_an_input());
    rig.pass = rig.pass.with_volume_from_full_attenuation();
    let outputs = rig.run(Panel::RESTING);
    assert_eq!(outputs.tick.monitor, monitored);
    assert!(
        !outputs
            .commands
            .as_slice()
            .iter()
            .any(|command| matches!(command, Command::SendAnalogWord(_))),
        "the knob cache survived, so no preamp code moved"
    );
}

#[test]
fn while_the_volume_is_held_at_full_a_pass_drops_its_volume_writes_and_keeps_its_other_commands() {
    let mut rig = Rig::booted(Panel::RESTING);
    rig.run(Panel::RESTING);
    let outputs = rig.run(
        Panel::RESTING
            .with(Knob::Main, 140)
            .with(Knob::PreampOne, 200),
    );
    assert!(!volume_writes(&outputs).is_empty());
    let held = outputs.holding_volume_at_full();
    assert!(volume_writes(&held).is_empty());
    let others: Vec<Command> = outputs
        .commands
        .as_slice()
        .iter()
        .copied()
        .filter(|command| !matches!(command, Command::WriteDac(DacWrite::Volume(..))))
        .collect();
    assert!(!others.is_empty());
    assert_eq!(held.commands.as_slice(), others.as_slice());
    assert_eq!(held.tick, outputs.tick);
    assert_eq!(held.analog, outputs.analog);
}

#[test]
fn a_mon_two_press_held_past_one_second_reports_one_long_press_and_toggles_no_monitoring() {
    let mut rig = Rig::booted(Panel::RESTING);
    let held = Panel {
        buttons: MONITOR_TWO_PRESSED,
        ..Panel::RESTING
    };
    let mut long_presses = 0;
    for _ in 0..300 {
        let outputs = rig.run(held);
        assert!(!outputs.long_presses[0], "MON 1 was never pressed");
        long_presses += usize::from(outputs.long_presses[1]);
    }
    let released = rig.run(Panel::RESTING);
    assert_eq!(long_presses, 1, "one long press per hold");
    assert!(!released.long_presses[1]);
    assert_eq!(
        released.tick.monitor,
        MonitorMix::OFF,
        "a long press is not a monitor toggle"
    );
}

#[test]
fn a_short_mon_two_press_reports_no_long_press() {
    let mut rig = Rig::booted(Panel::RESTING);
    for _ in 0..10 {
        let outputs = rig.run(Panel {
            buttons: MONITOR_TWO_PRESSED,
            ..Panel::RESTING
        });
        assert_eq!(outputs.long_presses, [false, false]);
    }
    assert_eq!(rig.run(Panel::RESTING).long_presses, [false, false]);
}
