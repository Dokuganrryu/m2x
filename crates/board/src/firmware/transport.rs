use device::quiesce::{self, BlockStop};

use super::clock_gates::{self, BusClock};
use super::map::{
    CTIMER0_TCR, CTIMER0_TCR_STOPPED, DMA0_ENABLECLR0, DMA0_FC0_TRANSMIT_CHANNEL,
    DMA0_FC2_TRANSMIT_CHANNEL, DMA0_FC6_RECEIVE_CHANNEL, DMA0_FC7_RECEIVE_CHANNEL,
    FLEXCOMM_PSELID_I2S_RECEIVE, FLEXCOMM_PSELID_I2S_TRANSMIT, FLEXCOMM_PSELID_PERSEL,
    FLEXCOMM0_PSELID, FLEXCOMM2_PSELID, FLEXCOMM6_PSELID, FLEXCOMM7_PSELID, I2S_CFG1_DISABLED,
    I2S0_CFG1, I2S2_CFG1, I2S6_CFG1, I2S7_CFG1,
};
use super::mmio::Register;

struct AudioFlexcomm {
    clock: BusClock,
    peripheral_select: Register,
    configuration: Register,
}

const AUDIO_FLEXCOMMS_IN_FACTORY_STOP_ORDER: [AudioFlexcomm; 4] = [
    AudioFlexcomm {
        clock: BusClock::Flexcomm7,
        peripheral_select: FLEXCOMM7_PSELID,
        configuration: I2S7_CFG1,
    },
    AudioFlexcomm {
        clock: BusClock::Flexcomm6,
        peripheral_select: FLEXCOMM6_PSELID,
        configuration: I2S6_CFG1,
    },
    AudioFlexcomm {
        clock: BusClock::Flexcomm2,
        peripheral_select: FLEXCOMM2_PSELID,
        configuration: I2S2_CFG1,
    },
    AudioFlexcomm {
        clock: BusClock::Flexcomm0,
        peripheral_select: FLEXCOMM0_PSELID,
        configuration: I2S0_CFG1,
    },
];

fn when_clocked(clock: BusClock, stop: impl FnOnce()) {
    match quiesce::bus_block_stop(clock_gates::is_on(clock)) {
        BlockStop::StopTheBlock => stop(),
        BlockStop::LeaveTheBlockAlone => {}
    }
}

pub fn stop_tick_timer() {
    when_clocked(BusClock::Timer0, || CTIMER0_TCR.write(CTIMER0_TCR_STOPPED));
}

pub fn stop_dma() {
    when_clocked(BusClock::Dma0, || {
        DMA0_ENABLECLR0.write(
            DMA0_FC0_TRANSMIT_CHANNEL
                | DMA0_FC2_TRANSMIT_CHANNEL
                | DMA0_FC6_RECEIVE_CHANNEL
                | DMA0_FC7_RECEIVE_CHANNEL,
        );
    });
}

fn selected_as_i2s(peripheral_select: Register) -> bool {
    matches!(
        peripheral_select.read() & FLEXCOMM_PSELID_PERSEL,
        FLEXCOMM_PSELID_I2S_TRANSMIT | FLEXCOMM_PSELID_I2S_RECEIVE
    )
}

pub fn stop_i2s() {
    for flexcomm in AUDIO_FLEXCOMMS_IN_FACTORY_STOP_ORDER {
        let clocked = clock_gates::is_on(flexcomm.clock);
        // WHY: The && reads PSELID only on a clocked Flexcomm, so an unclocked one sees no access at all
        match quiesce::i2s_stop(
            clocked,
            clocked && selected_as_i2s(flexcomm.peripheral_select),
        ) {
            BlockStop::StopTheBlock => flexcomm.configuration.write(I2S_CFG1_DISABLED),
            BlockStop::LeaveTheBlockAlone => {}
        }
    }
}

pub fn stop_transmit_dma() {
    when_clocked(BusClock::Dma0, || {
        DMA0_ENABLECLR0.write(DMA0_FC0_TRANSMIT_CHANNEL | DMA0_FC2_TRANSMIT_CHANNEL);
    });
}
