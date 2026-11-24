use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use device::fault::FaultCode;
use device::lcd::panel::{PanelCommand, RamWindow};
#[cfg(feature = "sound2-ui")]
use device::lcd::screen::{
    Batch, Glass, MOST_LCD_DESCRIPTORS, MOST_PIECES, RegionSet, render_batch,
};
use device::lcd::screen::{FAULT_WINDOW, FatalScreen, RegionPixels, Rgb565, Screen, render_fault};
#[cfg(not(feature = "sound2-ui"))]
use device::lcd::screen::{Region, render};
use device::lcd::wire::{
    MOST_COMMAND_WORDS, MOST_TRANSFERS_PER_DESCRIPTOR, PanelWord, WINDOW_HEADER_WORDS,
    command_words, descriptor_chunks, window_header,
};
use device::phantom::PinLevel;
use device::pin::Pin;

use super::clock_gates::{self, BusClock};
use super::clocks;
use super::dma::{self, Descriptor};
#[cfg(feature = "sound2-ui")]
use super::map::DMA0_CHANNEL_CFG_CHPRIORITY_LOWEST;
use super::map::{
    DMA0_ACTIVE0, DMA0_CHANNEL_CFG, DMA0_CHANNEL_CFG_INDEX_STEP, DMA0_CHANNEL_CFG_PERIPHREQEN,
    DMA0_CHANNEL_CFG_TRIGPOL, DMA0_CHANNEL_XFERCFG, DMA0_CHANNEL_XFERCFG_CFGVALID,
    DMA0_CHANNEL_XFERCFG_CLRTRIG, DMA0_CHANNEL_XFERCFG_INDEX_STEP, DMA0_CHANNEL_XFERCFG_RELOAD,
    DMA0_CHANNEL_XFERCFG_SWTRIG, DMA0_ENABLESET0, DMA0_FC4_TRANSMIT_CHANNEL,
    DMA0_FC4_TRANSMIT_CHANNEL_INDEX, DMA0_SETTRIG0, DMA0_XFERCFG_SOURCE_FIXED,
    DMA0_XFERCFG_SOURCE_STEPS_ONE_WIDTH, DMA0_XFERCFG_WIDTH_32_BITS,
    DMA0_XFERCFG_XFERCOUNT_EXHAUSTED, DMA0_XFERCFG_XFERCOUNT_SHIFT, FLEXCOMM_PSELID_SPI,
    FLEXCOMM4_PSELID, GPIO_DIRCLR, GPIO_W, GPIO_W_PIN_STEP, GPIO_W_READ_LOW,
    INPUTMUX_DMA0_REQ_ENA_SET, IOCON_FACTORY_DIGITAL_GPIO, IOCON_FACTORY_FC4_CLOCK_AND_DATA,
    IOCON_FACTORY_FC4_SELECT, IOCON_PIO0_1, IOCON_PIO0_4, IOCON_PIO0_5, IOCON_PIO0_18,
    IOCON_PIO0_19, IOCON_PIO1_6, IOCON_PIO1_24, SPI4_CFG, SPI4_CFG_FACTORY_MASTER_MODE_0, SPI4_DIV,
    SPI4_DIV_FACTORY_DIVIDE_BY_TWO, SPI4_FIFOCFG, SPI4_FIFOCFG_DMATX, SPI4_FIFOCFG_EMPTYRX,
    SPI4_FIFOCFG_EMPTYTX, SPI4_FIFOCFG_ENABLETX, SPI4_FIFOSTAT, SPI4_FIFOSTAT_TXEMPTY, SPI4_FIFOWR,
    SYSCON_AHBCLKCTRL0_MUX, SYSCON_AHBCLKCTRL1_FC4, SYSCON_AHBCLKCTRLSET_INDEX_STEP,
    SYSCON_AHBCLKCTRLSET0, SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE, SYSCON_FCCLKSEL4,
    SYSCON_PRESETCTRL1, SYSCON_PRESETCTRL1_FC4_RST,
};
use super::mmio::Register;
use super::pins;

const PANEL_RESET_OR_BACKLIGHT_PIN: Pin = Pin { port: 1, number: 6 };
const BACKLIGHT_ENABLE_PIN: Pin = Pin {
    port: 1,
    number: 24,
};
const TEARING_EFFECT_PIN_NUMBER: usize = 1;
const FULL_SCREEN_PIXELS: usize = RamWindow::FULL.pixel_count();
const MOST_CHAINED_DESCRIPTORS: usize = 24;
const BYTES_PER_WORD: u32 = 4;
const CLEAR_DESCRIPTORS: usize = WINDOW_HEADER_WORDS.div_ceil(MOST_TRANSFERS_PER_DESCRIPTOR)
    + (FULL_SCREEN_PIXELS - 1).div_ceil(MOST_TRANSFERS_PER_DESCRIPTOR)
    + 1;
const _: () = assert!(CLEAR_DESCRIPTORS <= MOST_CHAINED_DESCRIPTORS);
#[cfg(feature = "sound2-ui")]
const _: () = assert!(MOST_LCD_DESCRIPTORS == MOST_CHAINED_DESCRIPTORS);
#[cfg(feature = "sound2-ui")]
const BATCH_RUNS: usize = 2 * MOST_PIECES;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TearingEffect {
    Low,
    High,
}

#[derive(Clone, Copy)]
enum Source {
    Stepping,
    Fixed,
}

#[derive(Clone, Copy)]
struct Run {
    first_word: u32,
    words: usize,
    source: Source,
}

impl Run {
    fn of(words: &[PanelWord]) -> Self {
        Self {
            first_word: words.as_ptr() as u32,
            words: words.len(),
            source: Source::Stepping,
        }
    }

    fn repeating(word: &PanelWord, times: usize) -> Self {
        Self {
            first_word: core::ptr::from_ref(word) as u32,
            words: times,
            source: Source::Fixed,
        }
    }
}

#[repr(C)]
struct LinkMemory {
    chain: [Descriptor; MOST_CHAINED_DESCRIPTORS],
    header: [PanelWord; WINDOW_HEADER_WORDS],
    command: [PanelWord; MOST_COMMAND_WORDS],
    fill: [PanelWord; 2],
    pixels: RegionPixels,
    #[cfg(feature = "sound2-ui")]
    batch_headers: [[PanelWord; WINDOW_HEADER_WORDS]; MOST_PIECES],
}

struct LinkCell(UnsafeCell<MaybeUninit<LinkMemory>>);

unsafe impl Sync for LinkCell {}

#[unsafe(link_section = ".panel_buffers.lcd")]
static MEMORY: LinkCell = LinkCell(UnsafeCell::new(MaybeUninit::uninit()));

static CLAIMED: AtomicBool = AtomicBool::new(false);
static FATAL_SCREEN: AtomicU8 = AtomicU8::new(FatalScreen::NotStarted as u8);

pub struct LcdLink {
    memory: &'static mut LinkMemory,
}

impl LcdLink {
    pub fn claim() -> Option<Self> {
        if CLAIMED.swap(true, Ordering::AcqRel) {
            return None;
        }
        let slot = MEMORY.0.get();
        let memory = unsafe {
            slot.write_bytes(0, 1);
            (*slot).assume_init_mut()
        };
        Some(Self { memory })
    }

    // WHY: The fatal path masks every exception and never returns, so the thread never uses its link again
    pub fn for_the_fatal_screen() -> Option<Self> {
        let (next, draw) = FatalScreen::from_bits(FATAL_SCREEN.load(Ordering::Relaxed)).attempted();
        FATAL_SCREEN.store(next as u8, Ordering::Relaxed);
        draw.then(|| Self {
            memory: unsafe { (*MEMORY.0.get()).assume_init_mut() },
        })
    }

    pub fn configure_spi(&mut self) {
        clock_gates::enable(&[BusClock::Iocon, BusClock::Gpio0, BusClock::Dma0]);
        SYSCON_AHBCLKCTRLSET0.write(SYSCON_AHBCLKCTRL0_MUX);
        SYSCON_AHBCLKCTRLSET0
            .bytes_after(SYSCON_AHBCLKCTRLSET_INDEX_STEP)
            .write(SYSCON_AHBCLKCTRL1_FC4);
        // WHY: FCCLKSEL4 3 is FRO_HF_DIV, halted with its 96 MHz source off out of reset; the screen starts it here so it needs no earlier stage
        clocks::start_serial_bus_clock_source();
        SYSCON_FCCLKSEL4.write(SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE);
        SYSCON_PRESETCTRL1.set(SYSCON_PRESETCTRL1_FC4_RST);
        SYSCON_PRESETCTRL1.write(SYSCON_PRESETCTRL1.read() & !SYSCON_PRESETCTRL1_FC4_RST);
        FLEXCOMM4_PSELID.write(FLEXCOMM_PSELID_SPI);
        SPI4_CFG.write(SPI4_CFG_FACTORY_MASTER_MODE_0);
        SPI4_DIV.write(SPI4_DIV_FACTORY_DIVIDE_BY_TWO);
        SPI4_FIFOCFG.write(SPI4_FIFOCFG_EMPTYTX | SPI4_FIFOCFG_EMPTYRX);
        SPI4_FIFOCFG.write(SPI4_FIFOCFG_ENABLETX | SPI4_FIFOCFG_DMATX);
        IOCON_PIO0_4.write(IOCON_FACTORY_FC4_CLOCK_AND_DATA);
        IOCON_PIO0_5.write(IOCON_FACTORY_FC4_CLOCK_AND_DATA);
        IOCON_PIO0_18.write(IOCON_FACTORY_FC4_SELECT);
        IOCON_PIO0_19.write(IOCON_FACTORY_FC4_SELECT);
        IOCON_PIO0_1.write(IOCON_FACTORY_DIGITAL_GPIO);
        GPIO_DIRCLR.write(1 << TEARING_EFFECT_PIN_NUMBER);
        INPUTMUX_DMA0_REQ_ENA_SET.write(DMA0_FC4_TRANSMIT_CHANNEL);
        dma::enable_controller();
        DMA0_CHANNEL_CFG
            .bytes_after(DMA0_FC4_TRANSMIT_CHANNEL_INDEX * DMA0_CHANNEL_CFG_INDEX_STEP)
            .write(DMA0_CHANNEL_CFG_PERIPHREQEN | DMA0_CHANNEL_CFG_TRIGPOL | lcd_priority());
        // WHY: CFGVALID clear and XFERCOUNT exhausted, so the channel reads finished until the first start
        transfer_configuration().write(DMA0_XFERCFG_XFERCOUNT_EXHAUSTED);
        FATAL_SCREEN.store(
            FatalScreen::from_bits(FATAL_SCREEN.load(Ordering::Relaxed)).started() as u8,
            Ordering::Relaxed,
        );
    }

    pub fn drive_reset_or_backlight_high(&mut self) {
        clock_gates::enable(&[BusClock::Iocon, BusClock::Gpio1]);
        IOCON_PIO1_6.write(IOCON_FACTORY_DIGITAL_GPIO);
        pins::drive(PANEL_RESET_OR_BACKLIGHT_PIN, PinLevel::High);
    }

    pub fn drive_backlight_high(&mut self) {
        clock_gates::enable(&[BusClock::Iocon, BusClock::Gpio1]);
        IOCON_PIO1_24.write(IOCON_FACTORY_DIGITAL_GPIO);
        pins::drive(BACKLIGHT_ENABLE_PIN, PinLevel::High);
    }

    pub fn tearing_effect() -> TearingEffect {
        let level = GPIO_W
            .bytes_after(TEARING_EFFECT_PIN_NUMBER * GPIO_W_PIN_STEP)
            .read();
        if level == GPIO_W_READ_LOW {
            TearingEffect::Low
        } else {
            TearingEffect::High
        }
    }

    // WHY: UM11126 22.6.6, ACTIVE0 rises only once the operation has started, so ACTIVE0 low with an empty FIFO also reads a transfer that never ran as done; XFERCOUNT 0x3FF proves the last descriptor ran out
    pub fn finished(&self) -> bool {
        transfer_configuration().read() & DMA0_XFERCFG_XFERCOUNT_EXHAUSTED
            == DMA0_XFERCFG_XFERCOUNT_EXHAUSTED
            && DMA0_ACTIVE0.read() & DMA0_FC4_TRANSMIT_CHANNEL == 0
            && SPI4_FIFOSTAT.read() & SPI4_FIFOSTAT_TXEMPTY != 0
    }

    pub fn send_command(&mut self, command: PanelCommand) {
        let words = command_words(command, &mut self.memory.command);
        let run = Run::of(words);
        self.start(&[run]);
    }

    pub fn clear(&mut self, colour: Rgb565) {
        self.memory.header = window_header(RamWindow::FULL);
        self.memory.fill = [PanelWord::pixel(colour), PanelWord::pixel(colour).ending()];
        let header = Run::of(&self.memory.header);
        let body = Run::repeating(&self.memory.fill[0], FULL_SCREEN_PIXELS - 1);
        let last = Run::of(&self.memory.fill[1..]);
        self.start(&[header, body, last]);
    }

    #[cfg(feature = "sound2-ui")]
    pub fn send_batch(&mut self, dirty: RegionSet, screen: &Screen, glass: &mut Glass) -> Batch {
        let batch = render_batch(dirty, screen, glass, &mut self.memory.pixels);
        // WHY: Only runs[..count] reach the chain; the filler is a real buffer so no slot ever names an address outside the link memory
        let mut runs = [Run::of(&self.memory.header); BATCH_RUNS];
        let mut count = 0;
        for (piece, header) in batch
            .pieces
            .iter()
            .flatten()
            .zip(self.memory.batch_headers.iter_mut())
        {
            *header = window_header(piece.window);
            runs[count] = Run::of(header);
            runs[count + 1] = Run::of(self.memory.pixels.words(*piece));
            count += 2;
        }
        if count != 0 {
            self.start(&runs[..count]);
        }
        batch
    }

    #[cfg(not(feature = "sound2-ui"))]
    pub fn send_region(&mut self, region: Region, screen: &Screen) {
        self.memory.header = window_header(region.window());
        let pixels = Run::of(render(region, screen, &mut self.memory.pixels));
        let header = Run::of(&self.memory.header);
        self.start(&[header, pixels]);
    }

    pub fn send_fault(&mut self, code: FaultCode, stage: u8) {
        self.memory.header = window_header(FAULT_WINDOW);
        let pixels = Run::of(render_fault(code, stage, &mut self.memory.pixels));
        let header = Run::of(&self.memory.header);
        self.start(&[header, pixels]);
    }

    fn start(&mut self, runs: &[Run]) {
        let destination_end = SPI4_FIFOWR.address();
        let mut planned = [Descriptor::UNUSED; MOST_CHAINED_DESCRIPTORS + 1];
        let mut count = 0;
        for run in runs {
            for chunk in descriptor_chunks(run.words) {
                let (increment, source_end) = match run.source {
                    Source::Stepping => (
                        DMA0_XFERCFG_SOURCE_STEPS_ONE_WIDTH,
                        run.first_word + (chunk.first + chunk.count - 1) as u32 * BYTES_PER_WORD,
                    ),
                    Source::Fixed => (DMA0_XFERCFG_SOURCE_FIXED, run.first_word),
                };
                if let Some(slot) = planned.get_mut(count) {
                    *slot = Descriptor {
                        transfer: DMA0_CHANNEL_XFERCFG_CFGVALID
                            | DMA0_XFERCFG_WIDTH_32_BITS
                            | increment
                            | ((chunk.count - 1) as u32) << DMA0_XFERCFG_XFERCOUNT_SHIFT,
                        source_end,
                        destination_end,
                        next: 0,
                    };
                    count += 1;
                }
            }
        }
        let mut first = Descriptor::UNUSED;
        for (index, planned) in planned[..count].iter().enumerate() {
            let mut descriptor = *planned;
            if index + 1 < count {
                descriptor.transfer |= DMA0_CHANNEL_XFERCFG_RELOAD | DMA0_CHANNEL_XFERCFG_SWTRIG;
                descriptor.next = self.memory.chain[index].address();
            } else {
                descriptor.transfer |= DMA0_CHANNEL_XFERCFG_CLRTRIG;
            }
            match index.checked_sub(1) {
                None => first = descriptor,
                Some(link) => self.memory.chain[link] = descriptor,
            }
        }
        dma::set_head(
            DMA0_FC4_TRANSMIT_CHANNEL_INDEX,
            Descriptor {
                transfer: 0,
                ..first
            },
        );
        transfer_configuration().write(first.transfer);
        DMA0_ENABLESET0.write(DMA0_FC4_TRANSMIT_CHANNEL);
        DMA0_SETTRIG0.write(DMA0_FC4_TRANSMIT_CHANNEL);
    }
}

// WHY: UM11126 DMA CFG CHPRIORITY bits 18:16, 0 highest and 7 lowest; the audio channels at 0 then always win arbitration over the screen
#[cfg(feature = "sound2-ui")]
const fn lcd_priority() -> u32 {
    DMA0_CHANNEL_CFG_CHPRIORITY_LOWEST
}

#[cfg(not(feature = "sound2-ui"))]
const fn lcd_priority() -> u32 {
    0
}

fn transfer_configuration() -> Register {
    DMA0_CHANNEL_XFERCFG
        .bytes_after(DMA0_FC4_TRANSMIT_CHANNEL_INDEX * DMA0_CHANNEL_XFERCFG_INDEX_STEP)
}
