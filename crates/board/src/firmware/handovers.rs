use device::boot::BOOT_RATE;
use device::cycles::CycleSpans;
use device::input::Input;
use device::rate_change::ClockReport;
use device::usb::handover::{ClockReports, UsbNotices};
use dsp::{
    CaptureGainTargets, Gain, InputMute, InputMutes, LimiterCoefficients, LimiterSettings,
    LimiterTargets, MonitorMix, MonitorTargets, PublishCount, RateFamily, RateSwitch, SampleRate,
};

pub static CAPTURE_GAINS: CaptureGainTargets = CaptureGainTargets::new();
pub static MONITOR: MonitorTargets = MonitorTargets::new();
pub static INPUT_MUTES: InputMutes = InputMutes::new();
pub static LIMITER: LimiterTargets = LimiterTargets::new();
pub static RATE: RateSwitch = RateSwitch::at(BOOT_RATE);
pub static PUBLISHES: PublishCount = PublishCount::new();
pub static TICK_CYCLES: CycleSpans = CycleSpans::new(BOOT_RATE.family());
pub static USB_CYCLES: CycleSpans = CycleSpans::new(BOOT_RATE.family());
pub static USB_NOTICES: UsbNotices = UsbNotices::new();
pub static CLOCK: ClockReports = ClockReports::at(BOOT_RATE);

pub fn report_clock(report: ClockReport) {
    CLOCK.publish(report);
}

pub fn set_input(input: Input, mute: InputMute) {
    let [one, two] = INPUT_MUTES.latest();
    INPUT_MUTES.publish(match input {
        Input::One => [mute, two],
        Input::Two => [one, mute],
    });
    PUBLISHES.published();
}

pub fn publish_tick_gains(capture: [Gain; 2], monitor: MonitorMix) {
    CAPTURE_GAINS.publish(capture);
    MONITOR.publish(monitor);
    PUBLISHES.published();
}

pub fn retune_limiter(settings: LimiterSettings, family: RateFamily) {
    LIMITER.publish(LimiterCoefficients::design(settings, family));
    PUBLISHES.published();
}

pub fn switch_rate(rate: SampleRate, limiter: LimiterSettings) {
    LIMITER.publish(LimiterCoefficients::design(limiter, rate.family()));
    TICK_CYCLES.follow(rate.family());
    USB_CYCLES.follow(rate.family());
    RATE.switch_to(rate);
    PUBLISHES.published();
}

pub fn clear_dsp_state() {
    RATE.clear_dsp_state();
    PUBLISHES.published();
}
