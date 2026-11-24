use super::map::{
    SYSCON_AHBCLKCTRL0, SYSCON_AHBCLKCTRL0_DMA0, SYSCON_AHBCLKCTRL0_GPIO0,
    SYSCON_AHBCLKCTRL0_GPIO1, SYSCON_AHBCLKCTRL0_IOCON, SYSCON_AHBCLKCTRL1_FC0,
    SYSCON_AHBCLKCTRL1_FC1, SYSCON_AHBCLKCTRL1_FC2, SYSCON_AHBCLKCTRL1_FC3, SYSCON_AHBCLKCTRL1_FC6,
    SYSCON_AHBCLKCTRL1_FC7, SYSCON_AHBCLKCTRL1_TIMER0, SYSCON_AHBCLKCTRL2_ANALOG_CTRL,
    SYSCON_AHBCLKCTRL2_SYSCTL, SYSCON_AHBCLKCTRLSET_INDEX_STEP, SYSCON_AHBCLKCTRLSET0,
};
use super::mmio::Register;

#[derive(Clone, Copy)]
pub enum BusClock {
    Iocon,
    Gpio0,
    Gpio1,
    Dma0,
    Flexcomm0,
    Flexcomm1,
    Flexcomm2,
    Flexcomm3,
    Flexcomm6,
    Flexcomm7,
    Timer0,
    AnalogControl,
    SharedSignals,
}

impl BusClock {
    const fn set_register_index_and_bit(self) -> (usize, u32) {
        match self {
            BusClock::Iocon => (0, SYSCON_AHBCLKCTRL0_IOCON),
            BusClock::Gpio0 => (0, SYSCON_AHBCLKCTRL0_GPIO0),
            BusClock::Gpio1 => (0, SYSCON_AHBCLKCTRL0_GPIO1),
            BusClock::Dma0 => (0, SYSCON_AHBCLKCTRL0_DMA0),
            BusClock::Flexcomm0 => (1, SYSCON_AHBCLKCTRL1_FC0),
            BusClock::Flexcomm1 => (1, SYSCON_AHBCLKCTRL1_FC1),
            BusClock::Flexcomm2 => (1, SYSCON_AHBCLKCTRL1_FC2),
            BusClock::Flexcomm3 => (1, SYSCON_AHBCLKCTRL1_FC3),
            BusClock::Flexcomm6 => (1, SYSCON_AHBCLKCTRL1_FC6),
            BusClock::Flexcomm7 => (1, SYSCON_AHBCLKCTRL1_FC7),
            BusClock::Timer0 => (1, SYSCON_AHBCLKCTRL1_TIMER0),
            BusClock::AnalogControl => (2, SYSCON_AHBCLKCTRL2_ANALOG_CTRL),
            BusClock::SharedSignals => (2, SYSCON_AHBCLKCTRL2_SYSCTL),
        }
    }
}

impl BusClock {
    pub const fn status_register_and_bit(self) -> (Register, u32) {
        let (index, bit) = self.set_register_index_and_bit();
        (
            SYSCON_AHBCLKCTRL0.bytes_after(index * SYSCON_AHBCLKCTRLSET_INDEX_STEP),
            bit,
        )
    }

    pub const fn set_register_and_bit(self) -> (Register, u32) {
        let (index, bit) = self.set_register_index_and_bit();
        (
            SYSCON_AHBCLKCTRLSET0.bytes_after(index * SYSCON_AHBCLKCTRLSET_INDEX_STEP),
            bit,
        )
    }
}

pub fn is_on(clock: BusClock) -> bool {
    let (register, bit) = clock.status_register_and_bit();
    register.read() & bit != 0
}

pub fn enable(clocks: &[BusClock]) {
    for clock in clocks {
        let (register, bit) = clock.set_register_and_bit();
        register.write(bit);
    }
}
