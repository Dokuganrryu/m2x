use device::output_pin::OutputPin as BoardOutput;
use device::phantom::PinLevel;
use device::pin::Pin;

use super::clock_gates::{self, BusClock};
use super::map::{
    GPIO_DIRSET, GPIO_DIRSET_PORT_STEP, GPIO_W, GPIO_W_HIGH, GPIO_W_LOW, GPIO_W_PIN_STEP,
    GPIO_W_PINS, GPIO_W_PORT_STEP, GPIO_W_PORTS, GPIO_W_READ_LOW, IOCON_FACTORY_GPIO_OUTPUT_PAD,
    IOCON_PIO0_28, IOCON_PIO1_3, IOCON_PIO1_4, IOCON_PIO1_10, IOCON_PIO1_22, IOCON_PIO1_26,
    IOCON_PIO1_29,
};
use super::mmio::Register;

struct OutputPin {
    port: u8,
    number: u8,
}

impl OutputPin {
    const fn of(pin: Pin) -> Option<Self> {
        if pin.port < GPIO_W_PORTS && pin.number < GPIO_W_PINS {
            Some(Self {
                port: pin.port,
                number: pin.number,
            })
        } else {
            None
        }
    }

    const fn level_register(&self) -> Register {
        GPIO_W.bytes_after(
            self.port as usize * GPIO_W_PORT_STEP + self.number as usize * GPIO_W_PIN_STEP,
        )
    }

    const fn direction_set_register(&self) -> Register {
        GPIO_DIRSET.bytes_after(self.port as usize * GPIO_DIRSET_PORT_STEP)
    }
}

pub fn level(pin: Pin) -> Option<PinLevel> {
    let output = OutputPin::of(pin)?;
    let clock = if output.port == 0 {
        BusClock::Gpio0
    } else {
        BusClock::Gpio1
    };
    if !clock_gates::is_on(clock) {
        return None;
    }
    Some(if output.level_register().read() == GPIO_W_READ_LOW {
        PinLevel::Low
    } else {
        PinLevel::High
    })
}

pub fn drive(pin: Pin, level: PinLevel) {
    let Some(output) = OutputPin::of(pin) else {
        return;
    };
    output.level_register().write(match level {
        PinLevel::Low => GPIO_W_LOW,
        PinLevel::High => GPIO_W_HIGH,
    });
    output.direction_set_register().write(1 << output.number);
}

const fn pad_control(pin: BoardOutput) -> Register {
    match pin {
        BoardOutput::PhantomActive => IOCON_PIO1_4,
        BoardOutput::FirstBringUp => IOCON_PIO1_22,
        BoardOutput::SecondBringUp => IOCON_PIO1_26,
        BoardOutput::ThirdBringUp => IOCON_PIO1_10,
        BoardOutput::ConverterEnable => IOCON_PIO1_3,
        BoardOutput::AdcEnable => IOCON_PIO1_29,
        BoardOutput::TransportActive => IOCON_PIO0_28,
    }
}

pub fn select_factory_gpio_pad(pin: BoardOutput) {
    pad_control(pin).write(IOCON_FACTORY_GPIO_OUTPUT_PAD);
}
