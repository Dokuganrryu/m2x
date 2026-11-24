use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

use super::clock_gates::BusClock;
use super::interrupts::{DEVICE_INTERRUPTS, Handler, device_interrupts};
use super::map::{
    CTIMER0_TCR, CTIMER0_TCR_STOPPED, DMA0_ENABLECLR0, DMA0_EVERY_CHANNEL,
    DMA0_FC0_TRANSMIT_CHANNEL, DMA0_FC2_TRANSMIT_CHANNEL, GPIO_CLR, GPIO_CLR_PORT_STEP,
    GPIO_DIRCLR, GPIO_DIRCLR_PORT_STEP, GPIO_DIRSET, GPIO_DIRSET_PORT_STEP, GPIO_P0_28, GPIO_P1_3,
    GPIO_P1_4, GPIO_P1_24, GPIO_P1_25, GPIO_P1_29, IOCON_FACTORY_DIGITAL_GPIO,
    IOCON_FACTORY_GPIO_OUTPUT_PAD, IOCON_PIO1_3, IOCON_PIO1_4, IOCON_PIO1_24, IOCON_PIO1_25,
    IOCON_PIO1_29, NVIC_EVERY_INTERRUPT_IN_A_WORD, NVIC_ICER, NVIC_ICPR, NVIC_WORD_BYTES, SCB_CCR,
    SCB_CCR_STKOFHFNMIGN, SCB_ICSR, SCB_ICSR_PENDSTCLR, SCB_ICSR_PENDSVCLR, SCB_VTOR, SYST_CSR,
    SYST_CSR_STOPPED,
};
use super::mmio::{
    Register, data_synchronization_barrier, instruction_synchronization_barrier, quiesce_at_entry,
    unmask_interrupts,
};

#[repr(C)]
pub struct HandOverWrite {
    register: u32,
    keep: u32,
    set: u32,
}

const WRITE_ALL_BITS: u32 = 0;

const fn write(register: Register, value: u32) -> HandOverWrite {
    HandOverWrite {
        register: register.address(),
        keep: WRITE_ALL_BITS,
        set: value,
    }
}

const fn modify(register: Register, keep: u32, set: u32) -> HandOverWrite {
    HandOverWrite {
        register: register.address(),
        keep,
        set,
    }
}

const fn clock_on(clock: BusClock) -> HandOverWrite {
    let (register, bit) = clock.set_register_and_bit();
    write(register, bit)
}

// WHY: A register address is word aligned, so bit 0 marks a guard row: it reads the clock-gate word and, when the clock's bit is clear, skips the next rows, as quiesce_at_entry's interpreter does
#[cfg(feature = "milestone4")]
const GUARD_ROW: u32 = 1;

#[cfg(feature = "milestone4")]
const fn skip_unless_clocked(clock: BusClock, rows: u32) -> HandOverWrite {
    let (register, bit) = clock.status_register_and_bit();
    HandOverWrite {
        register: register.address() | GUARD_ROW,
        keep: bit,
        set: rows,
    }
}

const END_OF_WRITES: HandOverWrite = HandOverWrite {
    register: 0,
    keep: 0,
    set: 0,
};

#[cfg_attr(not(feature = "milestone2-nolimit"), unsafe(no_mangle))]
pub static HAND_OVER_WRITES: [HandOverWrite; 32] = [
    modify(SCB_CCR, !SCB_CCR_STKOFHFNMIGN, SCB_CCR_STKOFHFNMIGN),
    clock_on(BusClock::Gpio0),
    write(GPIO_CLR, GPIO_P0_28),
    write(GPIO_DIRSET, GPIO_P0_28),
    clock_on(BusClock::Iocon),
    clock_on(BusClock::Gpio1),
    write(IOCON_PIO1_24, IOCON_FACTORY_DIGITAL_GPIO),
    write(IOCON_PIO1_25, IOCON_FACTORY_DIGITAL_GPIO),
    write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), GPIO_P1_24),
    write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), GPIO_P1_24),
    write(GPIO_DIRCLR.bytes_after(GPIO_DIRCLR_PORT_STEP), GPIO_P1_25),
    write(IOCON_PIO1_4, IOCON_FACTORY_GPIO_OUTPUT_PAD),
    write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), GPIO_P1_4),
    write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), GPIO_P1_4),
    write(IOCON_PIO1_3, IOCON_FACTORY_GPIO_OUTPUT_PAD),
    write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), GPIO_P1_3),
    write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), GPIO_P1_3),
    write(IOCON_PIO1_29, IOCON_FACTORY_GPIO_OUTPUT_PAD),
    write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), GPIO_P1_29),
    write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), GPIO_P1_29),
    clock_on(BusClock::Dma0),
    write(
        DMA0_ENABLECLR0,
        DMA0_FC0_TRANSMIT_CHANNEL | DMA0_FC2_TRANSMIT_CHANNEL,
    ),
    write(DMA0_ENABLECLR0, DMA0_EVERY_CHANNEL),
    write(SYST_CSR, SYST_CSR_STOPPED),
    write(SCB_ICSR, SCB_ICSR_PENDSTCLR | SCB_ICSR_PENDSVCLR),
    clock_on(BusClock::Timer0),
    write(CTIMER0_TCR, CTIMER0_TCR_STOPPED),
    write(NVIC_ICER, NVIC_EVERY_INTERRUPT_IN_A_WORD),
    write(
        NVIC_ICER.bytes_after(NVIC_WORD_BYTES),
        NVIC_EVERY_INTERRUPT_IN_A_WORD,
    ),
    write(NVIC_ICPR, NVIC_EVERY_INTERRUPT_IN_A_WORD),
    write(
        NVIC_ICPR.bytes_after(NVIC_WORD_BYTES),
        NVIC_EVERY_INTERRUPT_IN_A_WORD,
    ),
    END_OF_WRITES,
];

const _: () = assert!(DEVICE_INTERRUPTS <= 2 * 32);

unsafe extern "C" {
    fn Reset();
    fn HardFault();
    fn DefaultHandler();
    fn __pre_init();
}

const CORE_VECTORS: usize = 16;

#[repr(C)]
struct RunningVectors {
    core: [Option<Handler>; CORE_VECTORS],
    device: [Handler; DEVICE_INTERRUPTS],
}

const RUNNING_VECTORS: RunningVectors = RunningVectors {
    core: [
        None,
        Some(Reset),
        Some(DefaultHandler),
        Some(HardFault),
        Some(DefaultHandler),
        Some(DefaultHandler),
        Some(DefaultHandler),
        Some(DefaultHandler),
        None,
        None,
        None,
        Some(DefaultHandler),
        Some(DefaultHandler),
        None,
        Some(DefaultHandler),
        Some(DefaultHandler),
    ],
    device: device_interrupts(),
};

pub const fn hand_over_handlers() -> [Handler; DEVICE_INTERRUPTS] {
    [__pre_init; DEVICE_INTERRUPTS]
}

#[repr(C, align(512))]
struct RunningVectorTable(UnsafeCell<MaybeUninit<RunningVectors>>);

unsafe impl Sync for RunningVectorTable {}

#[unsafe(link_section = ".running_vectors")]
static RUNNING_VECTOR_TABLE: RunningVectorTable =
    RunningVectorTable(UnsafeCell::new(MaybeUninit::uninit()));

pub fn serve_interrupts_from_the_running_table() {
    let table = RUNNING_VECTOR_TABLE.0.get();
    unsafe { table.write(MaybeUninit::new(RUNNING_VECTORS)) }
    data_synchronization_barrier();
    SCB_VTOR.write(table as u32);
    data_synchronization_barrier();
    instruction_synchronization_barrier();
    unmask_interrupts();
}

#[cfg(not(feature = "milestone2-nolimit"))]
quiesce_at_entry!(HAND_OVER_WRITES);

// WHY: The previous image sets no stack limit; cortex-m-rt's Reset sets MSPLIM and calls __pre_init next, so its first instruction clears it, and the table runs without the SCB_CCR row; the boot's deepest stack is 4,572 of 11,864 bytes
#[cfg(all(feature = "milestone2-nolimit", not(feature = "milestone4")))]
#[unsafe(no_mangle)]
pub static HAND_OVER_WRITES_WITHOUT_THE_STACK_LIMIT: [HandOverWrite; 31] = {
    assert!(HAND_OVER_WRITES[0].register == SCB_CCR.address());
    let mut rows = [END_OF_WRITES; 31];
    let mut at = 0;
    while at < rows.len() {
        let row = &HAND_OVER_WRITES[at + 1];
        rows[at] = HandOverWrite {
            register: row.register,
            keep: row.keep,
            set: row.set,
        };
        at += 1;
    }
    rows
};

#[cfg(all(feature = "milestone2-nolimit", not(feature = "milestone4")))]
quiesce_at_entry!(
    HAND_OVER_WRITES_WITHOUT_THE_STACK_LIMIT,
    "movs r0, #0",
    "msr msplim, r0"
);

#[cfg(feature = "milestone4")]
const fn is_clock_on_row(row: &HandOverWrite, clock: BusClock) -> bool {
    let (register, bit) = clock.set_register_and_bit();
    row.register == register.address() && row.keep == WRITE_ALL_BITS && row.set == bit
}

#[cfg(feature = "milestone4")]
const DMA0_STOP_ROWS: u32 = 3;
#[cfg(feature = "milestone4")]
const TIMER0_STOP_ROWS: u32 = 2;

// WHY: The loader hands over AHBCLKCTRL0 0x000EE1FB and AHBCLKCTRL1 0x00004000, so DMA0 and CTIMER0 are unclocked and cannot run; the stack-limit row is left out, and this build clocks and stops DMA0 and CTIMER0 only when the loader left their clocks on
#[cfg(feature = "milestone4")]
#[unsafe(no_mangle)]
pub static HAND_OVER_WRITES_GUARDED_BY_THE_CLOCKS: [HandOverWrite; 33] = {
    let source = &HAND_OVER_WRITES;
    assert!(source[0].register == SCB_CCR.address());
    let mut rows = [END_OF_WRITES; 33];
    let mut from = 1;
    let mut to = 0;
    let mut guards = 0;
    while from < source.len() {
        let row = &source[from];
        if is_clock_on_row(row, BusClock::Dma0) {
            rows[to] = skip_unless_clocked(BusClock::Dma0, DMA0_STOP_ROWS);
            to += 1;
            guards += 1;
        } else if is_clock_on_row(row, BusClock::Timer0) {
            rows[to] = skip_unless_clocked(BusClock::Timer0, TIMER0_STOP_ROWS);
            to += 1;
            guards += 1;
        }
        rows[to] = HandOverWrite {
            register: row.register,
            keep: row.keep,
            set: row.set,
        };
        to += 1;
        from += 1;
    }
    assert!(guards == 2 && to == rows.len() && rows[to - 1].register == 0);
    rows
};

#[cfg(feature = "milestone4")]
quiesce_at_entry!(
    HAND_OVER_WRITES_GUARDED_BY_THE_CLOCKS,
    "movs r0, #0",
    "msr msplim, r0"
);
