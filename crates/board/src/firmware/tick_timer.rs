use dsp::FramesPerTick;
use stream::tick_counter_start;

use super::audio_dma::with_the_input_mux_clock;
use super::audio_interrupts::enable_at_the_factory_audio_level;
use super::clock_gates::{self, BusClock};
use super::interrupts::CTIMER0_IRQ;
use super::map::{
    CTIMER0_CTCR, CTIMER0_CTCR_FACTORY_FALLING_EDGES_ON_CAP0, CTIMER0_IR, CTIMER0_MCR,
    CTIMER0_MCR_MR0I, CTIMER0_MCR_MR0R, CTIMER0_MCR_MR0RL, CTIMER0_MR0, CTIMER0_MSR0, CTIMER0_TC,
    CTIMER0_TCR, CTIMER0_TCR_CEN, CTIMER0_TCR_CRST, INPUTMUX_TIMER0CAPTSEL_FACTORY_SHARED_WS_0,
    INPUTMUX_TIMER0CAPTSEL0, SYSCON_CTIMERCLKSEL_SEL, SYSCON_CTIMERCLKSEL0,
    SYSCON_PRESETCTRL1_TIMER0_RST, SYSCON_PRESETCTRLCLR_INDEX_STEP, SYSCON_PRESETCTRLCLR0,
};

const PRESETCTRLCLR_INDEX_OF_TIMER0: usize = 1;

pub fn start(frames: FramesPerTick) {
    SYSCON_CTIMERCLKSEL0.write(SYSCON_CTIMERCLKSEL0.read() & !SYSCON_CTIMERCLKSEL_SEL);
    clock_gates::enable(&[BusClock::Timer0]);
    SYSCON_PRESETCTRLCLR0
        .bytes_after(PRESETCTRLCLR_INDEX_OF_TIMER0 * SYSCON_PRESETCTRLCLR_INDEX_STEP)
        .write(SYSCON_PRESETCTRL1_TIMER0_RST);
    with_the_input_mux_clock(|| {
        INPUTMUX_TIMER0CAPTSEL0.write(INPUTMUX_TIMER0CAPTSEL_FACTORY_SHARED_WS_0);
    });
    CTIMER0_CTCR.write(CTIMER0_CTCR_FACTORY_FALLING_EDGES_ON_CAP0);
    CTIMER0_MCR.write(CTIMER0_MCR_MR0I | CTIMER0_MCR_MR0R | CTIMER0_MCR_MR0RL);
    enable_at_the_factory_audio_level(CTIMER0_IRQ);
    CTIMER0_TCR.set(CTIMER0_TCR_CRST);
    CTIMER0_TCR.write(CTIMER0_TCR.read() & !CTIMER0_TCR_CRST);
    let frames_per_tick = frames.count() as u32;
    CTIMER0_MR0.write(frames_per_tick);
    CTIMER0_MSR0.write(frames_per_tick);
    CTIMER0_TC.write(tick_counter_start(frames));
    CTIMER0_TCR.write(CTIMER0_TCR_CEN);
}

#[inline(always)]
pub fn acknowledge() {
    CTIMER0_IR.write(CTIMER0_IR.read());
}
