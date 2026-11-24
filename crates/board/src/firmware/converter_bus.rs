use device::i2c::{BusAction, MasterStatus};

use core::sync::atomic::{AtomicBool, Ordering};

use super::clock_gates::{self, BusClock};
use super::clocks;
use super::map::{
    FLEXCOMM_PSELID_I2C, FLEXCOMM3_PSELID, I2C_MSTDAT_DATA_MASK, I2C_MSTSTATE_IDLE,
    I2C_MSTSTATE_NACK_ADDRESS, I2C_MSTSTATE_NACK_DATA, I2C_MSTSTATE_RECEIVE_READY,
    I2C_MSTSTATE_TRANSMIT_READY, I2C_STAT_MSTSTATE_MASK, I2C_STAT_MSTSTATE_SHIFT, I2C3_CFG,
    I2C3_CFG_MASTER_OFF, I2C3_CFG_MSTEN, I2C3_CLKDIV, I2C3_CLKDIV_FACTORY, I2C3_MSTCTL,
    I2C3_MSTCTL_MSTCONTINUE, I2C3_MSTCTL_MSTSTART, I2C3_MSTCTL_MSTSTOP, I2C3_MSTDAT, I2C3_MSTTIME,
    I2C3_MSTTIME_FACTORY, I2C3_STAT, I2C3_STAT_MSTARBLOSS, I2C3_STAT_MSTPENDING,
    I2C3_STAT_MSTSTSTPERR, IOCON_PIO0_3, IOCON_PIO0_3_FACTORY_FC3_SDA, IOCON_PIO0_21,
    IOCON_PIO0_21_FACTORY_FC3_SCL, SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE, SYSCON_FCCLKSEL3,
    SYSCON_PRESETCTRL1, SYSCON_PRESETCTRL1_FC3_RST,
};

const MASTER_ERRORS: u32 = I2C3_STAT_MSTARBLOSS | I2C3_STAT_MSTSTSTPERR;

static CONFIGURED: AtomicBool = AtomicBool::new(false);

pub fn configured() -> bool {
    CONFIGURED.load(Ordering::Acquire)
}

pub fn configure() {
    clock_gates::enable(&[BusClock::Iocon, BusClock::Flexcomm3]);
    clocks::start_serial_bus_clock_source();
    SYSCON_FCCLKSEL3.write(SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE);
    SYSCON_PRESETCTRL1.set(SYSCON_PRESETCTRL1_FC3_RST);
    SYSCON_PRESETCTRL1.write(SYSCON_PRESETCTRL1.read() & !SYSCON_PRESETCTRL1_FC3_RST);
    IOCON_PIO0_3.write(IOCON_PIO0_3_FACTORY_FC3_SDA);
    IOCON_PIO0_21.write(IOCON_PIO0_21_FACTORY_FC3_SCL);
    FLEXCOMM3_PSELID.write(FLEXCOMM_PSELID_I2C);
    I2C3_CLKDIV.write(I2C3_CLKDIV_FACTORY);
    I2C3_MSTTIME.write(I2C3_MSTTIME_FACTORY);
    I2C3_CFG.write(I2C3_CFG_MSTEN);
    CONFIGURED.store(true, Ordering::Release);
}

pub fn act(action: BusAction) {
    match action {
        BusAction::Transmit(byte) => I2C3_MSTDAT.write(u32::from(byte)),
        BusAction::Start => I2C3_MSTCTL.write(I2C3_MSTCTL_MSTSTART),
        BusAction::Continue => I2C3_MSTCTL.write(I2C3_MSTCTL_MSTCONTINUE),
        BusAction::Stop => I2C3_MSTCTL.write(I2C3_MSTCTL_MSTSTOP),
        BusAction::ResetMaster => {
            I2C3_CFG.write(I2C3_CFG_MASTER_OFF);
            I2C3_STAT.write(MASTER_ERRORS);
            I2C3_CFG.write(I2C3_CFG_MSTEN);
        }
    }
}

pub fn status() -> MasterStatus {
    let stat = I2C3_STAT.read();
    if stat & MASTER_ERRORS != 0 {
        return MasterStatus::Disrupted;
    }
    if stat & I2C3_STAT_MSTPENDING == 0 {
        return MasterStatus::InProgress;
    }
    match (stat >> I2C_STAT_MSTSTATE_SHIFT) & I2C_STAT_MSTSTATE_MASK {
        I2C_MSTSTATE_IDLE => MasterStatus::Idle,
        I2C_MSTSTATE_RECEIVE_READY => {
            MasterStatus::ReceiveReady((I2C3_MSTDAT.read() & I2C_MSTDAT_DATA_MASK) as u8)
        }
        I2C_MSTSTATE_TRANSMIT_READY => MasterStatus::TransmitReady,
        I2C_MSTSTATE_NACK_ADDRESS => MasterStatus::NackAddress,
        I2C_MSTSTATE_NACK_DATA => MasterStatus::NackData,
        _ => MasterStatus::Disrupted,
    }
}
