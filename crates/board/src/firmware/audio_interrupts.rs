use super::map::{
    NVIC_ICER, NVIC_ICPR, NVIC_IPR, NVIC_IPR_BYTE, NVIC_IPR_BYTES_PER_WORD, NVIC_ISER,
    NVIC_PRIORITY_FACTORY_AUDIO_LEVEL_1, NVIC_USB1_FACTORY_PRIORITY, NVIC_WORD_BYTES,
};

const _: () = assert!(
    NVIC_PRIORITY_FACTORY_AUDIO_LEVEL_1 == NVIC_USB1_FACTORY_PRIORITY,
    "the tick and the USB interrupt run at one level, so neither preempts the other"
);

const INTERRUPTS_PER_WORD: usize = 32;
const BITS_PER_BYTE: usize = 8;

pub fn enable_at_the_factory_audio_level(irq: usize) {
    let word = (irq / INTERRUPTS_PER_WORD) * NVIC_WORD_BYTES;
    let bit = 1 << (irq % INTERRUPTS_PER_WORD);
    NVIC_ICER.bytes_after(word).write(bit);
    NVIC_ICPR.bytes_after(word).write(bit);
    let priority = NVIC_IPR.bytes_after((irq / NVIC_IPR_BYTES_PER_WORD) * NVIC_IPR_BYTES_PER_WORD);
    let shift = (irq % NVIC_IPR_BYTES_PER_WORD) * BITS_PER_BYTE;
    priority.write(
        (priority.read() & !(NVIC_IPR_BYTE << shift))
            | (NVIC_PRIORITY_FACTORY_AUDIO_LEVEL_1 << shift),
    );
    NVIC_ISER.bytes_after(word).write(bit);
}
