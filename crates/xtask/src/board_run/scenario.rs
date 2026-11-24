use device::fault::{FaultCode, I2cPeer};

use super::gate::{
    AfterFault, AnalogWords, Expectation, ExpectedEnding, FenceProbe, MainClock, RateChange,
    Recorded, Silence,
};
use super::trace::Access;

pub struct Scenario {
    pub name: &'static str,
    pub board: &'static [&'static str],
    pub run_milliseconds: u64,
    pub expectation: Expectation,
    pub today: &'static str,
}

const fn probe(address: u32, access: Access, allowed: bool) -> FenceProbe {
    FenceProbe {
        address,
        access,
        allowed,
    }
}

// WHY: The fence allows the image and stack, the peripherals around the flash controller block and USB RAM, never fetches from USB RAM, faults both aliases of the block, flash and everything else, and the PPB is never checked
pub const FENCE: [FenceProbe; 19] = [
    probe(0x4003_4000, Access::Write, false),
    probe(0x4003_4000, Access::Read, false),
    probe(0x4003_4FFC, Access::Read, false),
    probe(0x5003_4000, Access::Read, false),
    probe(0x5003_4000, Access::Write, false),
    probe(0x0003_8200, Access::Read, false),
    probe(0x0003_8200, Access::Write, false),
    probe(0x4003_3FFC, Access::Write, true),
    probe(0x4003_5000, Access::Write, true),
    probe(0x4010_0000, Access::Write, true),
    probe(0x4010_3FFC, Access::Write, true),
    probe(0x4010_4000, Access::Write, true),
    probe(0x4010_0000, Access::Fetch, false),
    probe(0x2000_1000, Access::Write, true),
    probe(0x2002_FFFC, Access::Write, true),
    probe(0x2003_0000, Access::Write, false),
    probe(0x2000_1000, Access::Fetch, true),
    probe(0x4008_9800, Access::Fetch, false),
    probe(0xE000_ED94, Access::Write, true),
];

const BOOT_RUN: u64 = 3_000;
const SOAK_RUN: u64 = 10_000;

const STARVES_AFTER_USB_ATTACH: &str = "the boot runs B1 to B11, the knob scan answers, B12 ramps the DAC to the knob law target and the boot reaches Running with the loop and the feed alive";

const fn boots_to_running() -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: None,
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::NotRequired,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const DAC: FaultCode = FaultCode::Unanswered(I2cPeer::Dac);
const ADC: FaultCode = FaultCode::Unanswered(I2cPeer::Adc);
const EXPANDER: FaultCode = FaultCode::Unanswered(I2cPeer::Expander);

const CLOCKS: u8 = 2;
const USB_ATTACH: u8 = 3;
const ANALOG_OFF: u8 = 4;
const RAILS: u8 = 5;
const AUDIO_CLOCKS: u8 = 7;
const DAC_INIT: u8 = 11;
const ADC_INIT: u8 = 13;
const PANEL: u8 = 14;
const VOLUME_RAMP: u8 = 15;
const RUNNING: u8 = 16;

const QUIESCE_BATCH: (u8, u8) = (1, 12);
const CRYSTAL_READ: u8 = 1;
const CORE_PLL_LOCK_READ: u8 = 4;
const AFTER_THE_ALL_OFF_WORD: u8 = 2;
const AFTER_THE_CONVERTER_BUS_SET_UP: u8 = 10;

const fn boot_fault(record: Recorded) -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(record),
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::PanelAndUsb,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const fn dac_init_fault_after_the_noted_attenuation_skip() -> Expectation {
    boot_fault(Recorded::raised(DAC, DAC_INIT, 0))
}

const fn clock_fault(record: Recorded) -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(record),
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::PanelOnly,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MustStayPut,
    }
}

const fn panel_fault() -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(Recorded::noted(EXPANDER, PANEL, 0)),
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::AudioPlaying,
        silence: Silence::NotRequired,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const fn analog_word_refused() -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(Recorded::raised(
            FaultCode::RefusedCommand,
            ANALOG_OFF,
            AFTER_THE_ALL_OFF_WORD,
        )),
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::UsbOnly,
        silence: Silence::TransmitDmaOnly,
        analog_words: AnalogWords::NeverLatched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const fn ramp_dac_fault() -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(Recorded::raised(DAC, VOLUME_RAMP, 0)),
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const fn rate_change_pll_fault() -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(Recorded::raised(FaultCode::AudioPllUnlocked, RUNNING, 0)),
        quiet_after: None,
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::TransmitDmaOnly,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::NeverRestarts,
        main_clock: MainClock::MaySwitch,
    }
}

const fn parked_with(record: Recorded) -> Expectation {
    Expectation {
        ending: ExpectedEnding::Parked,
        record: Some(record),
        quiet_after: None,
        fence: &[],
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::NotRequired,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const fn parked_after_a_planted_store_in_running() -> Expectation {
    Expectation {
        ending: ExpectedEnding::Parked,
        record: Some(Recorded::raised(FaultCode::HardFault, RUNNING, 0)),
        quiet_after: None,
        fence: &[],
        planted_store: Some(0x4003_4000),
        after_fault: AfterFault::None,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

pub const SCENARIOS: [Scenario; 32] = [
    Scenario {
        name: "boot-adc-at-13",
        board: &["adc 13"],
        run_milliseconds: BOOT_RUN,
        expectation: boots_to_running(),
        today: STARVES_AFTER_USB_ATTACH,
    },
    Scenario {
        name: "boot-adc-at-4f",
        board: &["adc 4f"],
        run_milliseconds: BOOT_RUN,
        expectation: boots_to_running(),
        today: STARVES_AFTER_USB_ATTACH,
    },
    Scenario {
        name: "dac-never-acks",
        board: &["dac silent"],
        run_milliseconds: BOOT_RUN,
        expectation: dac_init_fault_after_the_noted_attenuation_skip(),
        today: "B5 moves on after four attempts per register as the boot allows and notes code 3, B8 fails with Unanswered(Dac), and the boot stores code 3, keeps P0_28 low, starts the screen and attaches USB",
    },
    Scenario {
        name: "dac-stops-acking-mid-init",
        board: &["dac silent-after 3"],
        run_milliseconds: BOOT_RUN,
        expectation: dac_init_fault_after_the_noted_attenuation_skip(),
        today: "the DAC answers three writes of B5 and then stops; B5 moves on and notes code 3, B7 starts the transport with P0_28 low, B8 fails with Unanswered(Dac) and stores code 3, and the degraded boot still starts the screen and attaches USB",
    },
    Scenario {
        name: "adc-absent",
        board: &["adc none"],
        run_milliseconds: BOOT_RUN,
        expectation: boots_to_running(),
        today: "both probes NACK, so the ADC is type 0 as the boot order says; the rest is as the nominal boot, F7 included",
    },
    Scenario {
        name: "adc-stops-acking-after-probe",
        board: &["adc 13 silent-after 1"],
        run_milliseconds: BOOT_RUN,
        expectation: boot_fault(Recorded::raised(ADC, ADC_INIT, 0)),
        today: "the ADC answers the 0x13 probe and then stops, so B9 fails with Unanswered(Adc); the boot records code 4, leaves P0_28 low and still starts the screen and attaches USB",
    },
    Scenario {
        name: "dac-skip-then-adc-stops-after-probe",
        board: &["dac silent-for 4", "adc 13 silent-after 1"],
        run_milliseconds: BOOT_RUN,
        expectation: boot_fault(Recorded::raised(ADC, ADC_INIT, 0)),
        today: "the DAC misses the four attempts of the first B5 attenuation write and answers from then on, so B5 moves on and notes code 3; the ADC answers the 0x13 probe and then stops, so B9 fails with Unanswered(Adc) and stores code 4, P0_28 stays low and the boot still starts the screen and attaches USB",
    },
    Scenario {
        name: "expander-absent",
        board: &["expander silent"],
        run_milliseconds: BOOT_RUN,
        expectation: panel_fault(),
        today: "B10 fails with Unanswered(Expander); the boot notes code 5 and stores no code, still starts the screen and the knob scan, attaches USB and plays, so only the expander's buttons and LEDs are lost",
    },
    Scenario {
        name: "crystal-never-ready",
        board: &["crystal never"],
        run_milliseconds: BOOT_RUN,
        expectation: clock_fault(Recorded::raised(
            FaultCode::CrystalNotReady,
            CLOCKS,
            CRYSTAL_READ,
        )),
        today: "the clock stage polls XO_READY until the 15,000,000-cycle budget runs out and reads 0, so the boot records code 12 before the core PLL, keeps the bootloader's clock, skips USB and the audio path and still sends the all-off word, raises the rails and starts the screen",
    },
    Scenario {
        name: "pll-lock-never-rises",
        board: &["pll-lock never"],
        run_milliseconds: BOOT_RUN,
        expectation: clock_fault(Recorded::raised(
            FaultCode::CorePllUnlocked,
            CLOCKS,
            CORE_PLL_LOCK_READ,
        )),
        today: "PLL1STAT.LOCK never rises, so the core PLL lock poll times out after 15,000,000 core cycles and the image never switches MAINCLKSELB to PLL1; the boot records code 13 on the bootloader's clock, skips USB and still starts the screen",
    },
    Scenario {
        name: "audio-pll-lock-never-rises",
        board: &["pll0-lock never"],
        run_milliseconds: BOOT_RUN,
        expectation: boot_fault(Recorded::raised(
            FaultCode::AudioPllUnlocked,
            AUDIO_CLOCKS,
            0,
        )),
        today: "the core runs on PLL1 and PLL0STAT.LOCK never rises, so the audio PLL poll times out after 15,000,000 core cycles; the boot records code 14 before MCLK or I2S can use PLL0, and USB and the screen, both started before the audio clocks, stay up",
    },
    Scenario {
        name: "usb-phy-pll-lock-never-rises",
        board: &["adc 13", "phy-pll-lock never"],
        run_milliseconds: BOOT_RUN,
        expectation: Expectation {
            record: Some(Recorded::noted(FaultCode::UsbPhyPllUnlocked, USB_ATTACH, 0)),
            ..boots_to_running()
        },
        today: "PLL_SIC.PLL_LOCK never rises, so the attach polls it for 15,000,000 core cycles after USBPHY.CTRL_SET, notes code 16 and still writes the DEVCMDSTAT connect; the rest is as the nominal boot",
    },
    Scenario {
        name: "clocks-fail-before-the-dac",
        board: &["pll-lock never"],
        run_milliseconds: BOOT_RUN,
        expectation: clock_fault(Recorded::raised(
            FaultCode::CorePllUnlocked,
            CLOCKS,
            CORE_PLL_LOCK_READ,
        )),
        today: "the clock stage starts PLL1 and its lock never rises, so the boot records code 13 before the rails, skips USB and the DAC and still raises the rails and starts the screen; the expander write of the panel stage goes out on an FC3 that the rails stage configured and no FC3 transfer precedes that configuration",
    },
    Scenario {
        name: "i2c-transfer-hangs",
        board: &["i2c-hang-from 0"],
        run_milliseconds: BOOT_RUN,
        expectation: dac_init_fault_after_the_noted_attenuation_skip(),
        today: "every bus 3 transfer hangs; each attempt times out after 10 ms, B5 moves on and notes code 3, both probes count as absent, B8 fails with Unanswered(Dac) and records code 3 raised, which replaces the note, B10 notes code 5 after it and changes nothing, and the degraded boot panels and attaches",
    },
    Scenario {
        name: "bus-fault-on-converter-bus-write",
        board: &["bus-fault-on-write 0x40089800 0"],
        run_milliseconds: BOOT_RUN,
        expectation: parked_with(Recorded::raised(
            FaultCode::HardFault,
            RAILS,
            AFTER_THE_CONVERTER_BUS_SET_UP,
        )),
        today: "a precise bus fault on the first FC3 CFG write escalates to HardFault; fault::enter silences, stores code 8 and parks",
    },
    Scenario {
        name: "factory-page-read-faults",
        board: &["factory-read-faults"],
        run_milliseconds: BOOT_RUN,
        expectation: boots_to_running(),
        today: "a blank factory page that faults on an AHB read is absorbed by the accessor's fault mask and the template serial is used; the rest is as the nominal boot, F7 included",
    },
    Scenario {
        name: "mpu-with-two-regions",
        board: &["mpu-regions 2"],
        run_milliseconds: BOOT_RUN,
        expectation: parked_with(Recorded::raised(
            FaultCode::FenceUnavailable,
            QUIESCE_BATCH.0,
            QUIESCE_BATCH.1,
        )),
        today: "the fence needs five regions, so B1 enters the fault path, stores code 10 and parks",
    },
    Scenario {
        name: "mpu-with-four-regions",
        board: &["mpu-regions 4"],
        run_milliseconds: BOOT_RUN,
        expectation: parked_with(Recorded::raised(
            FaultCode::FenceUnavailable,
            QUIESCE_BATCH.0,
            QUIESCE_BATCH.1,
        )),
        today: "one region short of the five the fence needs, so B1 enters the fault path, stores code 10 and parks",
    },
    Scenario {
        name: "mpu-ignores-region-writes",
        board: &["mpu-regions 8 write-locked"],
        run_milliseconds: BOOT_RUN,
        expectation: parked_with(Recorded::raised(
            FaultCode::FenceUnavailable,
            QUIESCE_BATCH.0,
            QUIESCE_BATCH.1,
        )),
        today: "a write-locked MPU keeps none of the fence's RBAR and RLAR words, so the read-back refuses the fence and B1 enters the fault path, stores code 10 and parks",
    },
    Scenario {
        name: "planted-store-to-the-flash-controller",
        board: &["plant-store 0x40034000 1000"],
        run_milliseconds: BOOT_RUN,
        expectation: Expectation {
            ending: ExpectedEnding::Parked,
            record: Some(Recorded::raised(FaultCode::HardFault, VOLUME_RAMP, 0)),
            quiet_after: None,
            fence: &[],
            planted_store: Some(0x4003_4000),
            after_fault: AfterFault::None,
            silence: Silence::WordAndTransmitDma,
            analog_words: AnalogWords::Latched,
            rate_change: RateChange::Ordered,
            main_clock: MainClock::MaySwitch,
        },
        today: "at 1000 ms, during the boot, the CPU leaves the image and stores to 0x4003_4000; the fence the image raised must fault it and fault::enter must park",
    },
    Scenario {
        name: "planted-store-after-running",
        board: &["adc 13", "plant-store 0x40034000 3200"],
        run_milliseconds: BOOT_RUN + 1_000,
        expectation: parked_after_a_planted_store_in_running(),
        today: "at 3200 ms, after B8 raised P0_28 and the boot reached Running, the CPU leaves the image and stores to 0x4003_4000; the fault routine must drop P0_28 before it parks",
    },
    Scenario {
        name: "inherited-watchdog-running",
        board: &["adc 13", "watchdog-inherited running 0x2000 0x800"],
        run_milliseconds: BOOT_RUN,
        expectation: boots_to_running(),
        today: "the bootloader left the watchdog running with a 32.8 ms timeout and a window of 0x800 counts; the claim reads MOD, opens the window and sets the longest timeout before its one feed, and the main loop feeds once per elapsed millisecond",
    },
    Scenario {
        name: "inherited-watchdog-running-then-fault",
        board: &[
            "adc 13",
            "watchdog-inherited running 0x2000 0x800",
            "plant-store 0x40034000 3200",
        ],
        run_milliseconds: BOOT_RUN + 1_000,
        expectation: parked_after_a_planted_store_in_running(),
        today: "the inherited running watchdog as above, then a planted store at 3200 ms faults; the park must keep feeding once per elapsed millisecond for the rest of the run",
    },
    Scenario {
        name: "inherited-watchdog-locked",
        board: &["adc 13", "watchdog-inherited locked 0xffffff 0xffffff"],
        run_milliseconds: BOOT_RUN,
        expectation: boots_to_running(),
        today: "the bootloader left the watchdog running under WDPROTECT with the longest timeout; a TC write or a feed would reset at once, so the claim and the main loop leave it untouched",
    },
    Scenario {
        name: "inherited-watchdog-locked-then-fault",
        board: &[
            "adc 13",
            "watchdog-inherited locked 0xffffff 0xffffff",
            "plant-store 0x40034000 3200",
        ],
        run_milliseconds: BOOT_RUN + 1_000,
        expectation: parked_after_a_planted_store_in_running(),
        today: "the inherited locked watchdog as above, then a planted store at 3200 ms faults; the park must leave it untouched for the rest of the run",
    },
    Scenario {
        name: "soak-ten-seconds",
        board: &["adc 13"],
        run_milliseconds: SOAK_RUN,
        expectation: boots_to_running(),
        today: "ten simulated seconds of main-loop passes with the panel idle; the main loop feeds once per elapsed millisecond whatever the host does",
    },
    Scenario {
        name: "charger-no-frames",
        board: &["host none"],
        run_milliseconds: BOOT_RUN + 5_000,
        expectation: boots_to_running(),
        today: "a charger: DCON is set but no frame ever comes, so M9 never reaches Configured and the feed needs no USB interrupt (R19); no reset in five seconds after attach",
    },
    Scenario {
        name: "host-changes-rate",
        board: &[
            "adc 13",
            "host-requests-rate 44100 at 3000",
            "host-requests-rate 192000 at 4500",
        ],
        run_milliseconds: 6_000,
        expectation: boots_to_running(),
        today: "after Running the host asks for 44.1 kHz at 3000 ms and 192 kHz at 4500 ms; the model's host sends only frames and cannot deliver a SET CUR control transfer, so each request is written straight into the USB_NOTICES rate word in RAM, the word the USB interrupt writes when it answers SET CUR",
    },
    Scenario {
        name: "analog-bus-never-answers",
        board: &["analog-bus never"],
        run_milliseconds: BOOT_RUN,
        expectation: analog_word_refused(),
        today: "FC1 never reports room for a frame, so the all-off word times out after USB attached and the boot records code 11 before the rails; the tick, DMA and I2S stops run before the fault code, USB stays attached and the panel stays dark because no rail rose",
    },
    Scenario {
        name: "dac-stops-acking-in-the-ramp",
        board: &["adc 13", "dac ramp-after 2"],
        run_milliseconds: BOOT_RUN,
        expectation: ramp_dac_fault(),
        today: "the DAC answers the boot, then two volume writes of B12 and nothing more; the ramp exhausts its attempts, latches silence, stops the tick, DMA and I2S and ends degraded with the outputs disconnected",
    },
    Scenario {
        name: "expander-then-ramp-dac",
        board: &["adc 13", "expander silent", "dac ramp-after 2"],
        run_milliseconds: BOOT_RUN,
        expectation: Expectation {
            ending: ExpectedEnding::LoopRunsOn,
            record: Some(Recorded::raised(DAC, VOLUME_RAMP, 0)),
            quiet_after: None,
            fence: &FENCE,
            planted_store: None,
            after_fault: AfterFault::None,
            silence: Silence::NotRequired,
            analog_words: AnalogWords::Latched,
            rate_change: RateChange::Ordered,
            main_clock: MainClock::MaySwitch,
        },
        today: "the expander never answers, so B10 notes code 5 and the boot plays; the ramp DAC then stops after two writes, the fault silences it and records code 3 at the ramp, which replaces the noted code 5",
    },
    Scenario {
        name: "rate-change-pll0-times-out",
        board: &[
            "adc 13",
            "pll0-lock never-after 1",
            "host-requests-rate 44100 at 3000",
        ],
        run_milliseconds: 5_000,
        expectation: rate_change_pll_fault(),
        today: "the boot's first PLL0 read sees LOCK and the host's 44.1 kHz request switches the family; PLL0STAT never locks again, so the rate change fails with code 14, the outputs never restart and the fault stops the transport",
    },
];

pub const RATE_CHANGE_FOLLOW_UP: &str = "host-changes-rate does not deliver SET CUR through the USB controller: the model's host sends only frames, so the scenario writes each request into the USB_NOTICES rate word in RAM, the word the USB interrupt writes when it answers SET CUR. Everything from that word on is the image's own code. The B8 P0_28 rule is judged over the whole trace: at each rise every volume register must hold 0xFF or a value no louder than before P0_28 fell, since the rate change attenuates only MainA to HeadphoneB.";
