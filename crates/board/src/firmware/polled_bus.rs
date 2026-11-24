use device::analog_word::{AnalogWord, SHIFT_BYTE};
use device::pll::{self, Observation, Readiness, Sample};
use device::serial_bus;

use super::clock_gates::{self, BusClock};
use super::clocks;
use super::dwt::{self, CycleClock};
use super::map::{
    FLEXCOMM_PSELID_SPI, FLEXCOMM1_PSELID, IOCON_PIO0_7, IOCON_PIO0_7_FACTORY_FC1_SCK,
    IOCON_PIO0_13, IOCON_PIO0_13_FACTORY_FC1_MOSI, IOCON_PIO0_14, IOCON_PIO0_14_FACTORY_FC1_SSEL1,
    SPI1_CFG, SPI1_CFG_FACTORY_MODE_3_MASTER, SPI1_DIV, SPI1_DIV_FACTORY, SPI1_FIFOCFG,
    SPI1_FIFOCFG_EMPTYRX, SPI1_FIFOCFG_EMPTYTX, SPI1_FIFOCFG_ENABLETX, SPI1_FIFOSTAT,
    SPI1_FIFOSTAT_TXEMPTY, SPI1_FIFOSTAT_TXNOTFULL, SPI1_FIFOWR, SPI1_FIFOWR_FACTORY_PAD_FRAME,
    SPI1_FIFOWR_FACTORY_SHIFT_BYTE_FRAME, SPI1_FIFOWR_FACTORY_WORD_FRAME, SPI1_STAT,
    SPI1_STAT_MSTIDLE, SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE, SYSCON_FCCLKSEL1,
    SYSCON_PRESETCTRL1, SYSCON_PRESETCTRL1_FC1_RST,
};

pub fn configure_analog_word_bus() {
    clock_gates::enable(&[BusClock::Iocon, BusClock::Flexcomm1]);
    clocks::start_serial_bus_clock_source();
    SYSCON_FCCLKSEL1.write(SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE);
    SYSCON_PRESETCTRL1.set(SYSCON_PRESETCTRL1_FC1_RST);
    SYSCON_PRESETCTRL1.write(SYSCON_PRESETCTRL1.read() & !SYSCON_PRESETCTRL1_FC1_RST);
    IOCON_PIO0_7.write(IOCON_PIO0_7_FACTORY_FC1_SCK);
    IOCON_PIO0_13.write(IOCON_PIO0_13_FACTORY_FC1_MOSI);
    IOCON_PIO0_14.write(IOCON_PIO0_14_FACTORY_FC1_SSEL1);
    FLEXCOMM1_PSELID.write(FLEXCOMM_PSELID_SPI);
    SPI1_CFG.write(SPI1_CFG_FACTORY_MODE_3_MASTER);
    SPI1_DIV.write(SPI1_DIV_FACTORY);
    SPI1_FIFOCFG.write(SPI1_FIFOCFG_EMPTYTX | SPI1_FIFOCFG_EMPTYRX);
    SPI1_FIFOCFG.write(SPI1_FIFOCFG_ENABLETX | SPI1_FIFOCFG_EMPTYTX);
}

fn fifo_has_room() -> bool {
    SPI1_FIFOSTAT.read() & SPI1_FIFOSTAT_TXNOTFULL != 0
}

fn word_is_latched() -> bool {
    let drained = SPI1_FIFOSTAT.read() & SPI1_FIFOSTAT_TXEMPTY != 0;
    let idle = SPI1_STAT.read() & SPI1_STAT_MSTIDLE != 0;
    drained && idle
}

fn await_acknowledgement(ready: impl Fn() -> bool) -> Readiness {
    let mut clock = CycleClock::start();
    pll::await_readiness(
        serial_bus::ACKNOWLEDGE_DEADLINE,
        || Observation {
            sample: if ready() {
                Sample::Ready
            } else {
                Sample::NotReady
            },
            at: clock.now(),
        },
        || {},
    )
}

#[inline(never)]
fn await_fifo_has_room() -> Readiness {
    await_acknowledgement(fifo_has_room)
}

#[inline(never)]
fn await_word_is_latched() -> Readiness {
    await_acknowledgement(word_is_latched)
}

#[inline(never)]
fn send_analog_word(word: AnalogWord) -> Readiness {
    dwt::ensure_the_cycle_counter_runs();
    for frame in [
        SPI1_FIFOWR_FACTORY_WORD_FRAME | u32::from(word.bits()),
        SPI1_FIFOWR_FACTORY_SHIFT_BYTE_FRAME | u32::from(SHIFT_BYTE),
        SPI1_FIFOWR_FACTORY_PAD_FRAME,
    ] {
        if await_fifo_has_room() == Readiness::TimedOut {
            return Readiness::TimedOut;
        }
        SPI1_FIFOWR.write(frame);
    }
    Readiness::Reached
}

pub fn send_analog_word_and_wait_until_latched(word: AnalogWord) -> Readiness {
    if send_analog_word(word) == Readiness::TimedOut {
        return Readiness::TimedOut;
    }
    await_word_is_latched()
}

pub fn send_analog_word_from_any_state_and_wait_until_latched(word: AnalogWord) -> Readiness {
    configure_analog_word_bus();
    send_analog_word_and_wait_until_latched(word)
}
