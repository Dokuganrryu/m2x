use device::rate_change::{ClockReport, ClockValidity, Event};
use device::supervisor::Usb;
use device::usb::handover::{ClockReports, NoticesSeen, UsbNotices};
use device::usb::request::DataTerminal;
use device::usb::service::Notice;
use dsp::SampleRate;

fn arrived(seen: &mut NoticesSeen, notices: &UsbNotices) -> Vec<Notice> {
    seen.take(notices).as_slice().to_vec()
}

#[test]
fn a_quiet_handover_delivers_nothing() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    assert_eq!(seen.bus_changed(&notices), None);
    assert!(seen.take(&notices).as_slice().is_empty());
}

#[test]
fn the_latest_requested_rate_wins_and_arrives_once() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::RateChange(Event::SetCur(SampleRate::Hz96000)));
    notices.publish(Notice::RateChange(Event::SetCur(SampleRate::Hz44100)));
    assert_eq!(
        arrived(&mut seen, &notices),
        [Notice::RateChange(Event::SetCur(SampleRate::Hz44100))]
    );
    assert!(arrived(&mut seen, &notices).is_empty());
}

#[test]
fn asking_again_for_the_same_rate_is_a_new_request() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::RateChange(Event::SetCur(SampleRate::Hz192000)));
    arrived(&mut seen, &notices);
    notices.publish(Notice::RateChange(Event::SetCur(SampleRate::Hz192000)));
    assert_eq!(
        arrived(&mut seen, &notices),
        [Notice::RateChange(Event::SetCur(SampleRate::Hz192000))]
    );
}

#[test]
fn a_silence_released_before_the_thread_looks_is_still_delivered_first() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::FadeOutputsToSilence);
    notices.publish(Notice::SuspendPhantom);
    notices.publish(Notice::ReleaseOutputSilence);
    notices.publish(Notice::ResumePhantom);
    assert_eq!(
        arrived(&mut seen, &notices),
        [
            Notice::FadeOutputsToSilence,
            Notice::ReleaseOutputSilence,
            Notice::SuspendPhantom,
            Notice::ResumePhantom,
        ]
    );
}

#[test]
fn a_held_silence_arrives_once() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::FadeOutputsToSilence);
    assert_eq!(arrived(&mut seen, &notices), [Notice::FadeOutputsToSilence]);
    assert!(arrived(&mut seen, &notices).is_empty());
}

#[test]
fn every_kind_of_notice_fits_one_pass() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    for notice in [
        Notice::FadeOutputsToSilence,
        Notice::ReleaseOutputSilence,
        Notice::SuspendPhantom,
        Notice::ResumePhantom,
        Notice::DataTerminal(DataTerminal::Present),
        Notice::DataTerminal(DataTerminal::Absent),
        Notice::RateChange(Event::SetCur(SampleRate::Hz88200)),
    ] {
        notices.publish(notice);
    }
    assert_eq!(arrived(&mut seen, &notices).len(), 7);
}

#[test]
fn notices_that_arrive_while_booting_are_discarded_and_never_replayed() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::FadeOutputsToSilence);
    notices.publish(Notice::RateChange(Event::SetCur(SampleRate::Hz96000)));
    notices.publish(Notice::SuspendPhantom);
    assert!(seen.drain(&notices, false).as_slice().is_empty());
    assert!(
        seen.drain(&notices, true).as_slice().is_empty(),
        "a discarded notice came back after the boot reached running"
    );
}

#[test]
fn a_notice_that_arrives_while_running_is_delivered_once() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::FadeOutputsToSilence);
    notices.publish(Notice::RateChange(Event::SetCur(SampleRate::Hz88200)));
    assert_eq!(
        seen.drain(&notices, true).as_slice(),
        [
            Notice::FadeOutputsToSilence,
            Notice::RateChange(Event::SetCur(SampleRate::Hz88200)),
        ]
    );
    assert!(seen.drain(&notices, true).as_slice().is_empty());
}

#[test]
fn the_bus_arrives_only_when_it_changes() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.bus(Usb::Silent);
    assert_eq!(seen.bus_changed(&notices), None);
    notices.bus(Usb::Serving);
    assert_eq!(seen.bus_changed(&notices), Some(Usb::Serving));
    assert_eq!(seen.bus_changed(&notices), None);
}

#[test]
fn the_clock_report_reads_back_what_the_thread_published() {
    let reports = ClockReports::at(SampleRate::Hz48000);
    assert_eq!(
        reports.latest(),
        ClockReport {
            rate: SampleRate::Hz48000,
            validity: ClockValidity::Valid,
        }
    );
    let changing = ClockReport {
        rate: SampleRate::Hz176400,
        validity: ClockValidity::Invalid,
    };
    reports.publish(changing);
    assert_eq!(reports.latest(), changing);
}
