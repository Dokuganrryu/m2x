use device::console::{
    Asked, BusHealth, ByteOut, ClockFacts, Lock, MainClock, PanelFacts, PowerFacts, Reply,
    bus_report, clocks_report, factory_serial_report, panel_report, power_report,
};
use device::control_pass::Monitoring;
use device::factory::{FACTORY_MAGIC, FactoryRead};
use device::i2c::Answer;
use device::phantom::{PhantomPower, PinLevel};
use device::usb::FACTORY_SERIAL_TEMPLATE;

struct Text(Vec<u8>);

impl ByteOut for Text {
    fn push(&mut self, byte: u8) -> bool {
        self.0.push(byte);
        true
    }
}

fn written(write: impl FnOnce(&mut Reply<'_, Text>)) -> String {
    let mut text = Text(Vec::new());
    write(&mut Reply::new(&mut text));
    String::from_utf8(text.0).unwrap_or_default()
}

#[test]
fn the_panel_report_names_buttons_leds_indicators_and_both_held_pins() {
    let facts = PanelFacts {
        buttons: 0xF7,
        leds: 0x32,
        phantom: [PhantomPower::On, PhantomPower::Off],
        monitoring: [Monitoring::Off, Monitoring::On],
        phantom_rail: Some(PinLevel::Low),
        output_connect: Some(PinLevel::Low),
    };
    assert_eq!(
        written(|reply| panel_report(reply, facts)),
        "panel buttons=247 leds=50 p48_1=on p48_2=off mon_1=off mon_2=on p1_4=low p0_28=low\r\n"
    );
}

#[test]
fn the_panel_report_says_when_a_pin_cannot_be_read() {
    let facts = PanelFacts {
        buttons: 0xFF,
        leds: 0x72,
        phantom: [PhantomPower::Off; 2],
        monitoring: [Monitoring::Off; 2],
        phantom_rail: None,
        output_connect: Some(PinLevel::High),
    };
    assert_eq!(
        written(|reply| panel_report(reply, facts)),
        "panel buttons=255 leds=114 p48_1=off p48_2=off mon_1=off mon_2=off p1_4=unclocked p0_28=high\r\n"
    );
}

#[test]
fn the_clocks_report_names_the_main_clock_and_both_plls() {
    let facts = ClockFacts {
        main: MainClock::Pll1,
        core_pll: Lock::Locked,
        audio_pll: Lock::Unlocked,
        #[cfg(feature = "sound3")]
        core: device::core_clock::CoreClock::Pll150,
    };
    #[cfg(not(feature = "sound3"))]
    assert_eq!(
        written(|reply| clocks_report(reply, facts)),
        "clocks main=pll1 pll1=locked pll0=unlocked\r\n"
    );
    #[cfg(feature = "sound3")]
    assert_eq!(
        written(|reply| clocks_report(reply, facts)),
        "clocks main=pll1 pll1=locked pll0=unlocked core=150MHz reason=raised tick_cycles=18750\r\n"
    );
}

#[test]
fn bus_health_counts_answers_and_keeps_the_last_peer_that_did_not_answer() {
    let health = BusHealth::START
        .after(0x39, Asked::Transfer, Answer::Acked)
        .after(0x49, Asked::Transfer, Answer::Nacked)
        .after(0x39, Asked::Transfer, Answer::Read(0xFF))
        .after(0x13, Asked::Transfer, Answer::Nacked);
    assert_eq!(
        health,
        BusHealth {
            answered: 2,
            unanswered: 2,
            last_unanswered: Some(0x13),
            absent: 0,
        }
    );
    assert_eq!(
        written(|reply| bus_report(reply, health)),
        "i2c answered=2 unanswered=2 last_unanswered=19 absent_probes=0\r\n"
    );
    assert_eq!(
        written(|reply| bus_report(reply, BusHealth::START)),
        "i2c answered=0 unanswered=0 last_unanswered=none absent_probes=0\r\n"
    );
}

#[test]
fn a_probe_nobody_answers_counts_as_an_absent_part_not_an_unanswered_transfer() {
    let health = BusHealth::START
        .after(0x13, Asked::Probe, Answer::Acked)
        .after(0x4F, Asked::Probe, Answer::Nacked);
    assert_eq!(
        written(|reply| bus_report(reply, health)),
        "i2c answered=1 unanswered=0 last_unanswered=none absent_probes=1\r\n"
    );
}

#[test]
fn the_power_report_prints_the_pmc_words_and_the_dcdc_voltage() {
    let facts = PowerFacts {
        dcdc0: Some(0x0108_0000 | (4 << 17)),
        dcdc1: Some(0x0000_1234),
        ldopmu: None,
    };
    assert_eq!(
        written(|reply| power_report(reply, facts)),
        "power dcdc0=0x01080000 vout_mv=1050 dcdc1=0x00001234 ldopmu=unreadable\r\n"
    );
}

#[test]
fn the_serial_report_shows_the_descriptor_serial_and_what_the_factory_page_holds() {
    let usb = FACTORY_SERIAL_TEMPLATE;
    let usb_text = String::from_utf8(usb.ascii().to_vec()).unwrap();
    let loaded = FactoryRead::Loaded {
        magic: FACTORY_MAGIC,
        serial: *b"M2AB012345",
    };
    assert_eq!(
        written(|reply| factory_serial_report(reply, &usb, loaded)),
        format!("serial usb={usb_text} factory=M2AB012345\r\n")
    );
    assert_eq!(
        written(|reply| factory_serial_report(reply, &usb, FactoryRead::Unreadable)),
        format!("serial usb={usb_text} factory=unreadable\r\n")
    );
    let blank = FactoryRead::Loaded {
        magic: 0,
        serial: [0; 10],
    };
    assert_eq!(
        written(|reply| factory_serial_report(reply, &usb, blank)),
        format!("serial usb={usb_text} factory=blank magic=0x00000000\r\n")
    );
}
