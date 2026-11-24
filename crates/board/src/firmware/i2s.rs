use dsp::FramesPerTick;

use super::audio_dma::{self, AudioChannel};
use super::clock_gates::{self, BusClock};
use super::map::{
    FLEXCOMM_PSELID_I2S_RECEIVE, FLEXCOMM_PSELID_I2S_TRANSMIT, FLEXCOMM_PSELID_PERSEL,
    FLEXCOMM0_PSELID, FLEXCOMM2_PSELID, FLEXCOMM6_PSELID, FLEXCOMM7_PSELID,
    I2S_CFG1_FACTORY_MASTER_CLASSIC_32_BIT, I2S_CFG1_FACTORY_SLAVE_CLASSIC_32_BIT,
    I2S_CFG2_FACTORY_64_BIT_FRAME, I2S_DIV_FACTORY_DOUBLE_SPEED, I2S_DIV_FACTORY_QUAD_SPEED,
    I2S_DIV_FACTORY_SINGLE_SPEED, I2S_FIFOCFG_FACTORY_FLUSHED_AND_OFF,
    I2S_FIFOCFG_FACTORY_RECEIVE_BY_DMA, I2S_FIFOCFG_FACTORY_TRANSMIT_BY_DMA_ZERO_WHEN_EMPTY,
    I2S0_CFG1, I2S0_CFG1_MAINENABLE, I2S0_CFG2, I2S0_DIV, I2S0_FIFOCFG, I2S0_FIFOCFG_TXI2SE0,
    I2S2_CFG1, I2S2_CFG2, I2S2_DIV, I2S2_FIFOCFG, I2S6_CFG1, I2S6_CFG2, I2S6_DIV, I2S6_FIFOCFG,
    I2S7_CFG1, I2S7_CFG2, I2S7_DIV, I2S7_FIFOCFG, IOCON_PIO0_20, IOCON_PIO0_20_FACTORY_FC7_DATA,
    IOCON_PIO0_24, IOCON_PIO0_24_FACTORY_FC0_DATA, IOCON_PIO0_26, IOCON_PIO0_26_FACTORY_FC2_DATA,
    IOCON_PIO1_12, IOCON_PIO1_12_FACTORY_FC6_SCK, IOCON_PIO1_13, IOCON_PIO1_13_FACTORY_FC6_DATA,
    IOCON_PIO1_28, IOCON_PIO1_28_FACTORY_FC7_SCK, IOCON_PIO1_30, IOCON_PIO1_30_FACTORY_FC7_WS,
    SYSCON_FCCLKSEL_FACTORY_PLL0_DIVIDED, SYSCON_FCCLKSEL_SEL, SYSCON_FCCLKSEL0, SYSCON_FCCLKSEL2,
    SYSCON_FCCLKSEL6, SYSCON_FCCLKSEL7, SYSCON_PRESETCTRL2, SYSCON_PRESETCTRL2_SYSCTL_RST,
    SYSCTL_FCCTRLSEL, SYSCTL_FCCTRLSEL_FACTORY_SCK_AND_WS_FROM_SHARED_SET_0,
    SYSCTL_FCCTRLSEL_INDEX_STEP, SYSCTL_SHAREDCTRLSET_FACTORY_SCK_AND_WS_FROM_FC7,
    SYSCTL_SHAREDCTRLSET0, SYSCTL_UPDATELCKOUT, SYSCTL_UPDATELCKOUT_FACTORY_UNLOCKED,
};
use super::mmio::Register;

const FACTORY_I2S_PAD_WORDS: [(Register, u32); 7] = [
    (IOCON_PIO1_28, IOCON_PIO1_28_FACTORY_FC7_SCK),
    (IOCON_PIO1_30, IOCON_PIO1_30_FACTORY_FC7_WS),
    (IOCON_PIO0_20, IOCON_PIO0_20_FACTORY_FC7_DATA),
    (IOCON_PIO1_12, IOCON_PIO1_12_FACTORY_FC6_SCK),
    (IOCON_PIO1_13, IOCON_PIO1_13_FACTORY_FC6_DATA),
    (IOCON_PIO0_26, IOCON_PIO0_26_FACTORY_FC2_DATA),
    (IOCON_PIO0_24, IOCON_PIO0_24_FACTORY_FC0_DATA),
];

const _: () = assert!(
    I2S_FIFOCFG_FACTORY_TRANSMIT_BY_DMA_ZERO_WHEN_EMPTY & I2S0_FIFOCFG_TXI2SE0 != 0,
    "a transmitter whose DMA stops sends zeros once its FIFO drains"
);

struct AudioFlexcomm {
    number: usize,
    clock_select: Register,
    peripheral_select: Register,
    role: u32,
    cfg1: Register,
    cfg1_word: u32,
    cfg2: Register,
    fifo: Register,
    fifo_running: u32,
    div: Register,
    channel: AudioChannel,
}

const FC7_MASTER_DRAINED: AudioFlexcomm = AudioFlexcomm {
    number: 7,
    clock_select: SYSCON_FCCLKSEL7,
    peripheral_select: FLEXCOMM7_PSELID,
    role: FLEXCOMM_PSELID_I2S_RECEIVE,
    cfg1: I2S7_CFG1,
    cfg1_word: I2S_CFG1_FACTORY_MASTER_CLASSIC_32_BIT,
    cfg2: I2S7_CFG2,
    fifo: I2S7_FIFOCFG,
    fifo_running: I2S_FIFOCFG_FACTORY_RECEIVE_BY_DMA,
    div: I2S7_DIV,
    channel: AudioChannel::ReceiveFromFc7,
};

const FC6_CAPTURE: AudioFlexcomm = AudioFlexcomm {
    number: 6,
    clock_select: SYSCON_FCCLKSEL6,
    peripheral_select: FLEXCOMM6_PSELID,
    role: FLEXCOMM_PSELID_I2S_RECEIVE,
    cfg1: I2S6_CFG1,
    cfg1_word: I2S_CFG1_FACTORY_SLAVE_CLASSIC_32_BIT,
    cfg2: I2S6_CFG2,
    fifo: I2S6_FIFOCFG,
    fifo_running: I2S_FIFOCFG_FACTORY_RECEIVE_BY_DMA,
    div: I2S6_DIV,
    channel: AudioChannel::ReceiveFromFc6,
};

const FC2_PLAYBACK: AudioFlexcomm = AudioFlexcomm {
    number: 2,
    clock_select: SYSCON_FCCLKSEL2,
    peripheral_select: FLEXCOMM2_PSELID,
    role: FLEXCOMM_PSELID_I2S_TRANSMIT,
    cfg1: I2S2_CFG1,
    cfg1_word: I2S_CFG1_FACTORY_SLAVE_CLASSIC_32_BIT,
    cfg2: I2S2_CFG2,
    fifo: I2S2_FIFOCFG,
    fifo_running: I2S_FIFOCFG_FACTORY_TRANSMIT_BY_DMA_ZERO_WHEN_EMPTY,
    div: I2S2_DIV,
    channel: AudioChannel::TransmitToFc2,
};

const FC0_PLAYBACK_MIRROR: AudioFlexcomm = AudioFlexcomm {
    number: 0,
    clock_select: SYSCON_FCCLKSEL0,
    peripheral_select: FLEXCOMM0_PSELID,
    role: FLEXCOMM_PSELID_I2S_TRANSMIT,
    cfg1: I2S0_CFG1,
    cfg1_word: I2S_CFG1_FACTORY_SLAVE_CLASSIC_32_BIT,
    cfg2: I2S0_CFG2,
    fifo: I2S0_FIFOCFG,
    fifo_running: I2S_FIFOCFG_FACTORY_TRANSMIT_BY_DMA_ZERO_WHEN_EMPTY,
    div: I2S0_DIV,
    channel: AudioChannel::TransmitToFc0,
};

const IN_FACTORY_CONFIGURATION_ORDER: [AudioFlexcomm; 4] = [
    FC7_MASTER_DRAINED,
    FC6_CAPTURE,
    FC2_PLAYBACK,
    FC0_PLAYBACK_MIRROR,
];

const IN_FACTORY_ENABLE_ORDER: [AudioFlexcomm; 4] = [
    FC0_PLAYBACK_MIRROR,
    FC2_PLAYBACK,
    FC6_CAPTURE,
    FC7_MASTER_DRAINED,
];

const fn factory_div(frames: FramesPerTick) -> u32 {
    match frames {
        FramesPerTick::Six => I2S_DIV_FACTORY_SINGLE_SPEED,
        FramesPerTick::Twelve => I2S_DIV_FACTORY_DOUBLE_SPEED,
        FramesPerTick::TwentyFour => I2S_DIV_FACTORY_QUAD_SPEED,
    }
}

fn replace_field(register: Register, field: u32, value: u32) {
    register.write((register.read() & !field) | value);
}

fn route_the_shared_clocks_from_fc7() {
    clock_gates::enable(&[
        BusClock::Iocon,
        BusClock::Flexcomm7,
        BusClock::Flexcomm6,
        BusClock::Flexcomm2,
        BusClock::Flexcomm0,
        BusClock::Dma0,
        BusClock::SharedSignals,
    ]);
    for flexcomm in &IN_FACTORY_CONFIGURATION_ORDER {
        replace_field(
            flexcomm.clock_select,
            SYSCON_FCCLKSEL_SEL,
            SYSCON_FCCLKSEL_FACTORY_PLL0_DIVIDED,
        );
    }
    SYSCON_PRESETCTRL2.write(SYSCON_PRESETCTRL2.read() & !SYSCON_PRESETCTRL2_SYSCTL_RST);
    SYSCTL_UPDATELCKOUT.write(SYSCTL_UPDATELCKOUT_FACTORY_UNLOCKED);
    for flexcomm in &IN_FACTORY_ENABLE_ORDER {
        SYSCTL_FCCTRLSEL
            .bytes_after(flexcomm.number * SYSCTL_FCCTRLSEL_INDEX_STEP)
            .write(SYSCTL_FCCTRLSEL_FACTORY_SCK_AND_WS_FROM_SHARED_SET_0);
    }
    SYSCTL_SHAREDCTRLSET0.write(SYSCTL_SHAREDCTRLSET_FACTORY_SCK_AND_WS_FROM_FC7);
}

fn mux_the_factory_i2s_pads() {
    for (pad, word) in FACTORY_I2S_PAD_WORDS {
        pad.write(word);
    }
}

pub fn start(frames: FramesPerTick) {
    route_the_shared_clocks_from_fc7();
    mux_the_factory_i2s_pads();
    for flexcomm in &IN_FACTORY_CONFIGURATION_ORDER {
        replace_field(
            flexcomm.peripheral_select,
            FLEXCOMM_PSELID_PERSEL,
            flexcomm.role,
        );
        flexcomm.cfg1.write(flexcomm.cfg1_word);
        flexcomm.cfg2.write(I2S_CFG2_FACTORY_64_BIT_FRAME);
        flexcomm.fifo.write(I2S_FIFOCFG_FACTORY_FLUSHED_AND_OFF);
        flexcomm.div.write(factory_div(frames));
    }
    audio_dma::zero_transmit_buffers();
    for flexcomm in &IN_FACTORY_ENABLE_ORDER {
        audio_dma::start_channel(flexcomm.channel);
        flexcomm.fifo.write(flexcomm.fifo_running);
        flexcomm.cfg1.set(I2S0_CFG1_MAINENABLE);
    }
}
