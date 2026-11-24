use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use dsp::{
    Band, BandFrequency, BandGain, BandQ, Ceiling, CoefficientExchange, CoefficientSet, FilterKind,
    Gain, InputMute, InputMutes, LimiterCoefficients, LimiterFollower, LimiterSettings,
    LimiterTargets, MonitorMix, MonitorTargets, PeqPreset, PresetBand, PublishCount, PublishesSeen,
    RateFamily, RateFollower, RateSwitch, ReleaseTime, SampleRate,
};

const BLEND_A: MonitorMix = MonitorMix {
    playback: Gain::from_q5_27(0x0100_0000),
    inputs: [Gain::from_q5_27(0x0200_0000), Gain::from_q5_27(0x0300_0000)],
};
const BLEND_B: MonitorMix = MonitorMix {
    playback: Gain::from_q5_27(0x0700_0000),
    inputs: [Gain::from_q5_27(0x0600_0000), Gain::from_q5_27(0x0500_0000)],
};

static BOOT_MONITOR: MonitorTargets = MonitorTargets::new();
static BOOT_MUTES: InputMutes = InputMutes::new();
static BOOT_RATE: RateSwitch = RateSwitch::at(SampleRate::Hz48000);
static BOOT_LIMITER: LimiterTargets = LimiterTargets::new();

fn designed(tenths: i16, millis: u16, family: RateFamily) -> LimiterCoefficients {
    LimiterCoefficients::design(
        LimiterSettings {
            ceiling: Ceiling::from_tenths_of_dbfs(tenths).unwrap_or_default(),
            release: ReleaseTime::from_millis(millis).unwrap_or_default(),
        },
        family,
    )
}

#[test]
fn the_tick_keeps_its_limiter_until_thread_mode_publishes_one() {
    let mut follower = LimiterFollower::of(&BOOT_LIMITER);
    assert_eq!(follower.changed(&BOOT_LIMITER), None);
}

#[test]
fn the_tick_takes_each_published_limiter_once() {
    let targets = LimiterTargets::new();
    let mut follower = LimiterFollower::of(&targets);
    let soft = designed(-30, 400, RateFamily::FortyEightKilohertz);
    let hard = designed(-5, 20, RateFamily::FortyFourPointOneKilohertz);
    targets.publish(soft);
    assert_eq!(follower.changed(&targets), Some(soft));
    assert_eq!(follower.changed(&targets), None);
    targets.publish(hard);
    targets.publish(soft);
    targets.publish(hard);
    assert_eq!(follower.changed(&targets), Some(hard));
    assert_eq!(follower.changed(&targets), None);
}

#[test]
fn a_follower_made_after_a_publish_starts_from_it() {
    let targets = LimiterTargets::new();
    targets.publish(designed(-30, 400, RateFamily::FortyEightKilohertz));
    let mut follower = LimiterFollower::of(&targets);
    assert_eq!(follower.changed(&targets), None);
}

#[test]
fn monitoring_starts_off_and_the_tick_takes_the_latest_published_mix() {
    assert_eq!(BOOT_MONITOR.latest_or(BLEND_B), MonitorMix::OFF);
    let targets = MonitorTargets::new();
    targets.publish(BLEND_A);
    assert_eq!(targets.latest_or(MonitorMix::OFF), BLEND_A);
    targets.publish(BLEND_B);
    targets.publish(BLEND_A);
    assert_eq!(targets.latest_or(BLEND_B), BLEND_A);
}

#[test]
fn a_tick_never_sees_the_playback_gain_of_one_publish_with_the_input_gains_of_another() {
    let targets = Arc::new(MonitorTargets::new());
    targets.publish(BLEND_A);
    let rounds = 10_000u64;
    let after_publish = Arc::new(Barrier::new(2));
    let after_read = Arc::new(Barrier::new(2));
    let writer = {
        let (targets, after_publish, after_read) = (
            Arc::clone(&targets),
            Arc::clone(&after_publish),
            Arc::clone(&after_read),
        );
        std::thread::spawn(move || {
            for round in 0..rounds {
                targets.publish(if round % 2 == 0 { BLEND_B } else { BLEND_A });
                after_publish.wait();
                after_read.wait();
            }
        })
    };
    let mut held = BLEND_A;
    let mut changes = 0u64;
    for _ in 0..rounds {
        after_publish.wait();
        let next = targets.latest_or(held);
        assert!(
            next == BLEND_A || next == BLEND_B,
            "a torn monitor mix {next:?}"
        );
        changes += u64::from(next != held);
        held = next;
        after_read.wait();
    }
    writer.join().expect("the writer finishes");
    assert!(changes > 0, "the reader saw every publish land");

    let targets = Arc::new(MonitorTargets::new());
    targets.publish(BLEND_A);
    let done = Arc::new(AtomicBool::new(false));
    let writer = {
        let (targets, done) = (Arc::clone(&targets), Arc::clone(&done));
        std::thread::spawn(move || {
            for round in 0..400_000 {
                targets.publish(if round % 2 == 0 { BLEND_B } else { BLEND_A });
            }
            done.store(true, Ordering::Release);
        })
    };
    let mut held = BLEND_A;
    while !done.load(Ordering::Acquire) {
        let next = targets.latest_or(held);
        assert!(
            next == BLEND_A || next == BLEND_B,
            "a torn monitor mix {next:?}"
        );
        held = next;
    }
    writer.join().expect("the writer finishes");
}

#[test]
fn both_inputs_start_live_and_each_mute_is_published_on_its_own_input() {
    assert_eq!(BOOT_MUTES.latest(), [InputMute::Live; 2]);
    let mutes = InputMutes::new();
    mutes.publish([InputMute::Muted, InputMute::Live]);
    assert_eq!(mutes.latest(), [InputMute::Muted, InputMute::Live]);
    mutes.publish([InputMute::Live, InputMute::Muted]);
    assert_eq!(mutes.latest(), [InputMute::Live, InputMute::Muted]);
}

#[test]
fn a_follower_sees_nothing_until_the_thread_switches_the_rate_and_then_sees_it_once() {
    let mut follower = RateFollower::of(&BOOT_RATE);
    assert_eq!(follower.changed(&BOOT_RATE), None);
    let switch = RateSwitch::at(SampleRate::Hz48000);
    let mut follower = RateFollower::of(&switch);
    switch.switch_to(SampleRate::Hz96000);
    assert_eq!(follower.changed(&switch), Some(SampleRate::Hz96000));
    assert_eq!(follower.changed(&switch), None);
}

#[test]
fn a_dsp_state_clear_reaches_the_follower_at_the_rate_already_switched_to() {
    let switch = RateSwitch::at(SampleRate::Hz44100);
    let mut follower = RateFollower::of(&switch);
    switch.clear_dsp_state();
    assert_eq!(follower.changed(&switch), Some(SampleRate::Hz44100));
    switch.switch_to(SampleRate::Hz192000);
    switch.clear_dsp_state();
    assert_eq!(follower.changed(&switch), Some(SampleRate::Hz192000));
    assert_eq!(follower.changed(&switch), None);
}

#[test]
fn every_rate_survives_the_handover_and_two_followers_each_see_the_switch() {
    let switch = RateSwitch::at(SampleRate::Hz48000);
    let mut tick = RateFollower::of(&switch);
    let mut usb = RateFollower::of(&switch);
    for rate in SampleRate::ALL {
        switch.switch_to(rate);
        assert_eq!(tick.changed(&switch), Some(rate));
        assert_eq!(usb.changed(&switch), Some(rate));
    }
}

#[test]
fn a_rate_change_redesigns_the_current_preset_for_the_new_rate() {
    let mut preset = PeqPreset::FLAT;
    preset.bands[0] = PresetBand {
        band: Band {
            kind: FilterKind::Peaking,
            frequency: BandFrequency::from_hz(1_000.0).expect("a valid frequency"),
            gain: BandGain::from_db(6.0).expect("a valid gain"),
            q: BandQ::new(1.0).expect("a valid Q"),
        },
        enabled: true,
    };
    let mut exchange = CoefficientExchange::flat_at(SampleRate::Hz48000);
    let (mut publisher, mut tick) = exchange.split();
    publisher.change(&preset, SampleRate::Hz48000);
    tick.adopt();
    publisher.poll();
    publisher.change_rate(SampleRate::Hz88200);
    let adopted = tick.adopt();
    let expected = CoefficientSet::design(&preset, SampleRate::Hz88200, adopted.generation());
    assert_eq!(adopted.rate(), SampleRate::Hz88200);
    assert_eq!(adopted.bands(), expected.bands());
    assert_eq!(adopted.enabled_mask(), expected.enabled_mask());
}

#[test]
fn the_tick_learns_of_a_publish_once_and_of_nothing_when_thread_mode_published_nothing() {
    let count = PublishCount::new();
    let mut seen = PublishesSeen::of(&count);
    assert!(!seen.any_new(&count));
    count.published();
    count.published();
    assert!(seen.any_new(&count));
    assert!(!seen.any_new(&count));
    count.published();
    assert!(seen.any_new(&count));
}

#[test]
fn a_publish_counted_after_its_data_is_seen_with_its_data() {
    for _ in 0..200 {
        let count = Arc::new(PublishCount::new());
        let gains = Arc::new(dsp::CaptureGainTargets::new());
        let mut seen = PublishesSeen::of(&count);
        let writer = {
            let (count, gains) = (Arc::clone(&count), Arc::clone(&gains));
            std::thread::spawn(move || {
                gains.publish([BLEND_A.playback, BLEND_B.playback]);
                count.published();
            })
        };
        while !seen.any_new(&count) {}
        assert_eq!(gains.latest(), [BLEND_A.playback, BLEND_B.playback]);
        writer.join().expect("the writer finishes");
    }
}
