use crate::pin::{PHANTOM_ACTIVE_PIN, Pin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPin {
    PhantomActive,
    FirstBringUp,
    SecondBringUp,
    ThirdBringUp,
    ConverterEnable,
    AdcEnable,
    TransportActive,
}

impl OutputPin {
    pub const ALL: [OutputPin; 7] = [
        OutputPin::PhantomActive,
        OutputPin::FirstBringUp,
        OutputPin::SecondBringUp,
        OutputPin::ThirdBringUp,
        OutputPin::ConverterEnable,
        OutputPin::AdcEnable,
        OutputPin::TransportActive,
    ];

    pub const fn pin(self) -> Pin {
        match self {
            OutputPin::PhantomActive => PHANTOM_ACTIVE_PIN,
            OutputPin::FirstBringUp => Pin {
                port: 1,
                number: 22,
            },
            OutputPin::SecondBringUp => Pin {
                port: 1,
                number: 26,
            },
            OutputPin::ThirdBringUp => Pin {
                port: 1,
                number: 10,
            },
            OutputPin::ConverterEnable => Pin { port: 1, number: 3 },
            OutputPin::AdcEnable => Pin {
                port: 1,
                number: 29,
            },
            OutputPin::TransportActive => Pin {
                port: 0,
                number: 28,
            },
        }
    }
}
