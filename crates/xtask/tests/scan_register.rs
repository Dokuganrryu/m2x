use xtask::scan::{Formed, RegisterEvent, RegisterValue};

const STATES: [RegisterValue; 3] = [
    RegisterValue::Unknown,
    RegisterValue::Known(0x2002_00c8),
    RegisterValue::HighHalfOnly(0x4003),
];

const EVENTS: [RegisterEvent; 5] = [
    RegisterEvent::MoveWide(0x4010),
    RegisterEvent::MoveTop(0x4003),
    RegisterEvent::Load(0x0003_8200),
    RegisterEvent::AddImmediate(4),
    RegisterEvent::Clobber,
];

#[test]
fn every_state_and_event_pair_has_its_pinned_successor_and_output() {
    use Formed as F;
    use RegisterValue::{HighHalfOnly as High, Known, Unknown};
    let expected: [[(RegisterValue, Option<Formed>); 5]; 3] = [
        [
            (Known(0x4010), None),
            (High(0x4003), Some(F::HighHalfOnly(0x4003))),
            (Known(0x0003_8200), None),
            (Unknown, None),
            (Unknown, None),
        ],
        [
            (Known(0x4010), None),
            (Known(0x4003_00c8), Some(F::Value(0x4003_00c8))),
            (Known(0x0003_8200), None),
            (Known(0x2002_00cc), Some(F::Value(0x2002_00cc))),
            (Unknown, None),
        ],
        [
            (Known(0x4010), None),
            (High(0x4003), Some(F::HighHalfOnly(0x4003))),
            (Known(0x0003_8200), None),
            (High(0x4003), None),
            (Unknown, None),
        ],
    ];
    for (state, row) in STATES.into_iter().zip(expected) {
        for (event, pinned) in EVENTS.into_iter().zip(row) {
            assert_eq!(state.next(event), pinned, "{state:?} on {event:?}");
        }
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn event(&mut self) -> RegisterEvent {
        let value = self.next();
        match value % 5 {
            0 => RegisterEvent::MoveWide(value as u16),
            1 => RegisterEvent::MoveTop((value >> 8) as u16),
            2 => RegisterEvent::Load(value as u32),
            3 => RegisterEvent::AddImmediate(i64::from((value % 0x2000) as i32 - 0x1000)),
            _ => RegisterEvent::Clobber,
        }
    }
}

fn random_walks(
    mut each_step: impl FnMut(RegisterValue, RegisterEvent, RegisterValue, Option<Formed>),
) {
    for seed in 1..=64 {
        let mut random = Lcg(seed);
        let mut state = RegisterValue::Unknown;
        for _ in 0..500 {
            let event = random.event();
            let (next, formed) = state.next(event);
            each_step(state, event, next, formed);
            state = next;
        }
    }
}

#[test]
fn every_movt_reports_what_it_formed_so_no_address_is_built_silently() {
    random_walks(|_, event, next, formed| {
        if let RegisterEvent::MoveTop(high) = event {
            match formed {
                Some(Formed::Value(value)) => {
                    assert_eq!(value >> 16, u32::from(high));
                    assert_eq!(next, RegisterValue::Known(value));
                }
                Some(Formed::HighHalfOnly(reported)) => assert_eq!(reported, high),
                None => panic!("movt of {high:#x} formed nothing"),
            }
        }
    });
}

#[test]
fn a_reported_value_is_always_the_value_the_register_now_holds() {
    random_walks(|_, _, next, formed| {
        if let Some(Formed::Value(value)) = formed {
            assert_eq!(next, RegisterValue::Known(value));
        }
    });
}

#[test]
fn movw_then_movt_forms_the_whole_address_whatever_the_register_held_before() {
    random_walks(|state, _, _, _| {
        let (after_low, _) = state.next(RegisterEvent::MoveWide(0x4000));
        let (after_high, formed) = after_low.next(RegisterEvent::MoveTop(0x4003));
        assert_eq!(after_high, RegisterValue::Known(0x4003_4000));
        assert_eq!(formed, Some(Formed::Value(0x4003_4000)));
    });
}

#[test]
fn a_clobber_always_forgets_the_value() {
    random_walks(|_, event, next, formed| {
        if event == RegisterEvent::Clobber {
            assert_eq!((next, formed), (RegisterValue::Unknown, None));
        }
    });
}
