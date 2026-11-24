use super::interrupts::DEVICE_INTERRUPTS;
use super::map::{NVIC_EVERY_INTERRUPT_IN_A_WORD, NVIC_ICER, NVIC_ICPR, NVIC_WORD_BYTES};

const INTERRUPTS_PER_WORD: usize = 32;
const DEVICE_INTERRUPT_WORDS: usize = DEVICE_INTERRUPTS.div_ceil(INTERRUPTS_PER_WORD);

pub fn disable_and_clear_every_device_interrupt() {
    for word in 0..DEVICE_INTERRUPT_WORDS {
        NVIC_ICER
            .bytes_after(word * NVIC_WORD_BYTES)
            .write(NVIC_EVERY_INTERRUPT_IN_A_WORD);
    }
    for word in 0..DEVICE_INTERRUPT_WORDS {
        NVIC_ICPR
            .bytes_after(word * NVIC_WORD_BYTES)
            .write(NVIC_EVERY_INTERRUPT_IN_A_WORD);
    }
}
