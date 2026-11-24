use device::breadcrumb::{Breadcrumb, Place, Weight};
use device::console::{ByteOut, Reply, fault_report, knob_report, limiter_report, serial_report};
use device::knob::{KnobReading, KnobReadings, Knobs};
use device::usb::SerialNumber;
use dsp::{Ceiling, LimiterSettings, ReleaseTime};

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
fn the_knob_report_names_the_five_cached_positions() {
    let knobs = Knobs::first_scan(KnobReadings([
        KnobReading(0x0000),
        KnobReading(0xF0FF),
        KnobReading(0x7800),
        KnobReading(0x1200),
        KnobReading(0x0100),
    ]));
    assert_eq!(
        written(|reply| knob_report(reply, Some(knobs))),
        "knob pre1=18 pre2=0 main=240 hp=1 mix=120\r\n"
    );
}

#[test]
fn the_knob_report_says_so_before_the_first_scan() {
    assert_eq!(
        written(|reply| knob_report(reply, None)),
        "knob unscanned\r\n"
    );
}

#[test]
fn the_serial_report_prints_the_factory_copy() {
    let serial = SerialNumber::from_factory_copy(*b"M2B1234567");
    assert_eq!(
        written(|reply| serial_report(reply, &serial)),
        "serial M2B1234567\r\n"
    );
}

#[test]
fn the_limiter_report_shows_the_ceiling_and_release() {
    let settings = LimiterSettings {
        ceiling: Ceiling::from_tenths_of_dbfs(-15).unwrap_or_default(),
        release: ReleaseTime::from_millis(250).unwrap_or_default(),
    };
    assert_eq!(
        written(|reply| limiter_report(reply, settings)),
        "limiter ceiling=-1.5 release=250\r\n"
    );
}

#[test]
fn the_default_limiter_report_shows_the_default_settings() {
    let settings = LimiterSettings::default();
    let line = written(|reply| limiter_report(reply, settings));
    assert!(line.starts_with("limiter ceiling=-"), "{line}");
    assert!(line.ends_with("\r\n"), "{line}");
}

#[test]
fn fault_prints_the_breadcrumb_and_the_previous_one() {
    let no_tick_in_dac_init = Breadcrumb {
        place: Place { stage: 11, step: 4 },
        code: 15,
        weight: Weight::Raised,
        latest: 17,
    };
    assert_eq!(
        written(|reply| fault_report(reply, no_tick_in_dac_init, None)),
        "fault at=11.4 code=15 latest=17 prev=none\r\n"
    );
    let running = Breadcrumb {
        place: Place { stage: 16, step: 0 },
        code: 0,
        weight: Weight::Noted,
        latest: 16,
    };
    let refused_output_connect = Breadcrumb {
        place: Place { stage: 12, step: 0 },
        code: 11,
        weight: Weight::Raised,
        latest: 17,
    };
    assert_eq!(
        written(|reply| fault_report(reply, running, Some(refused_output_connect))),
        "fault at=16.0 code=0 latest=16 prev_at=12.0 prev_code=11 prev_latest=17\r\n"
    );
    let raised_16 = Breadcrumb::unpack(0x1100_0000 | 1 << 23 | 16 << 16 | 2 << 8 | 3);
    let noted_16 = Breadcrumb::unpack(0x1000_0000 | 16 << 16 | 2 << 8 | 3);
    assert_eq!(
        written(|reply| fault_report(reply, raised_16, Some(noted_16))),
        "fault at=3.2 code=16 latest=17 prev_at=3.2 prev_code=16 prev_latest=16\r\n",
        "the weight bit is not part of the printed code"
    );
}
