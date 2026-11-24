#[cfg(feature = "sound1")]
mod boot_board;

#[cfg(feature = "sound1")]
mod sound1 {
    use super::boot_board::{Bench, Caller, Seen, run_image};
    use device::boot::{Image, State, VolumeTargets};
    use device::command::Command;
    use device::dac::{Attenuation, DacWrite, VolumeRegister};
    use device::output_pin::OutputPin;
    use device::phantom::PinLevel;
    use device::silent::phantom_held;

    const ENOUGH_PASSES: usize = 20_000;

    fn commands(caller: &Caller) -> Vec<Command> {
        caller
            .trace
            .iter()
            .filter_map(|seen| match seen {
                Seen::Command(command) => Some(phantom_held(*command)),
                Seen::Waited(_) | Seen::Log(_) | Seen::StartLcd => None,
            })
            .collect()
    }

    fn first(issued: &[Command], wanted: impl Fn(&Command) -> bool) -> usize {
        issued.iter().position(wanted).expect("the boot issues it")
    }

    fn is_dac(command: &Command) -> bool {
        matches!(command, Command::WriteDac(_) | Command::ReadDac(_))
    }

    fn clean() -> Caller {
        run_image(Image::Full, Bench::ALL_ACK, ENOUGH_PASSES)
    }

    #[test]
    fn the_converters_are_powered_before_the_first_dac_transfer_as_factory_does() {
        let issued = commands(&clean());
        let converter = first(&issued, |command| {
            *command == Command::DrivePin(OutputPin::ConverterEnable, PinLevel::High)
        });
        let adc = first(&issued, |command| {
            *command == Command::DrivePin(OutputPin::AdcEnable, PinLevel::High)
        });
        let dac = first(&issued, is_dac);
        assert!(
            converter < dac && adc < dac,
            "P1_3 {converter}, P1_29 {adc}, first DAC {dac}"
        );
        let probe = first(&issued, |command| matches!(command, Command::ProbeAdc(_)));
        assert!(dac < probe, "the attenuation writes go before the probes");
    }

    #[test]
    fn a_clean_bench_reaches_running_with_no_fault_recorded() {
        let caller = clean();
        assert!(
            matches!(caller.boot, State::Running(..)),
            "{:?}",
            caller.boot
        );
        assert!(
            caller.records().is_empty(),
            "records {:?}",
            caller.records()
        );
    }

    #[test]
    fn output_connect_rises_only_after_every_volume_register_is_fully_attenuated() {
        let issued = commands(&clean());
        let connect = first(&issued, |command| {
            *command == Command::DrivePin(OutputPin::TransportActive, PinLevel::High)
        });
        for register in VolumeRegister::ALL {
            let muted = issued[..connect].iter().rposition(|command| {
                matches!(command, Command::WriteDac(DacWrite::Volume(r, _)) if *r == register)
            });
            let level = muted.map(|at| match issued[at] {
                Command::WriteDac(DacWrite::Volume(_, level)) => level,
                _ => unreachable!(),
            });
            assert_eq!(
                level,
                Some(Attenuation::FULL),
                "{register:?} before P0_28 rises"
            );
        }
    }

    #[test]
    fn after_connect_main_and_headphone_ramp_from_full_attenuation_half_a_decibel_a_pass_to_the_knobs()
     {
        let bench = Bench {
            targets: VolumeTargets {
                main: Attenuation::from_half_decibel_steps(0x20),
                headphone: Attenuation::from_half_decibel_steps(0x40),
            },
            ..Bench::ALL_ACK
        };
        let caller = run_image(Image::Full, bench, ENOUGH_PASSES);
        assert!(
            matches!(caller.boot, State::Running(..)),
            "{:?}",
            caller.boot
        );
        let issued = commands(&caller);
        let connect = first(&issued, |command| {
            *command == Command::DrivePin(OutputPin::TransportActive, PinLevel::High)
        });
        for (register, target) in [
            (VolumeRegister::MainA, 0x20),
            (VolumeRegister::MainB, 0x20),
            (VolumeRegister::HeadphoneA, 0x40),
            (VolumeRegister::HeadphoneB, 0x40),
        ] {
            let levels: Vec<u8> = issued[connect..]
                .iter()
                .filter_map(|command| match command {
                    Command::WriteDac(DacWrite::Volume(r, level)) if *r == register => {
                        Some(level.half_decibel_steps())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                levels.first(),
                Some(&0xFE),
                "{register:?} starts one step under 0xFF"
            );
            assert_eq!(
                levels.last(),
                Some(&target),
                "{register:?} ends at the knob"
            );
            assert!(
                levels.windows(2).all(|pair| pair[0] - pair[1] == 1),
                "{register:?} moves one half-decibel step per write"
            );
        }
    }

    #[test]
    fn the_phantom_rail_and_phantom_bits_never_leave_the_filter() {
        for command in commands(&clean()) {
            assert_ne!(
                command,
                Command::DrivePin(OutputPin::PhantomActive, PinLevel::High)
            );
            if let Command::SendAnalogWord(word) = command {
                assert_eq!(word.bits() & ((1 << 14) | (1 << 6)), 0);
            }
        }
    }
}
