use core::sync::atomic::{AtomicU32, Ordering};

use stream::{DmaBuffer, DmaFill, DmaTransport, XferCount};

use super::clock_gates::{self, BusClock};
use super::dma::{self, Descriptor};
use super::map::{
    DMA0_CHANNEL_CFG, DMA0_CHANNEL_CFG_FACTORY_PERIPHERAL_REQUEST, DMA0_CHANNEL_CFG_INDEX_STEP,
    DMA0_CHANNEL_CTLSTAT, DMA0_CHANNEL_CTLSTAT_FACTORY_CLEARED, DMA0_CHANNEL_CTLSTAT_INDEX_STEP,
    DMA0_CHANNEL_XFERCFG, DMA0_CHANNEL_XFERCFG_FACTORY_RECEIVE_96_FRAMES,
    DMA0_CHANNEL_XFERCFG_FACTORY_TRANSMIT_96_FRAMES, DMA0_CHANNEL_XFERCFG_INDEX_STEP,
    DMA0_CHANNEL_XFERCFG_SWTRIG, DMA0_ENABLESET0, DMA0_FC0_TRANSMIT_CHANNEL,
    DMA0_FC0_TRANSMIT_CHANNEL_NUMBER, DMA0_FC2_TRANSMIT_CHANNEL, DMA0_FC2_TRANSMIT_CHANNEL_NUMBER,
    DMA0_FC6_RECEIVE_CHANNEL, DMA0_FC6_RECEIVE_CHANNEL_NUMBER, DMA0_FC7_RECEIVE_CHANNEL,
    DMA0_FC7_RECEIVE_CHANNEL_NUMBER, DMA0_SETTRIG0, I2S0_FIFOWR, I2S2_FIFOWR, I2S6_FIFORD,
    I2S7_FIFORD, INPUTMUX_DMA0_REQ_ENA_SET, SYSCON_AHBCLKCTRL0_MUX, SYSCON_AHBCLKCTRLCLR0,
    SYSCON_AHBCLKCTRLSET0,
};
use super::mmio::Register;

pub static AUDIO: DmaTransport = DmaTransport::silent();
static RECEIVE_FROM_FC7_DRAINED: DmaBuffer = DmaBuffer::silent();

#[repr(C, align(16))]
struct Reload {
    transfer: AtomicU32,
    source_end: AtomicU32,
    destination_end: AtomicU32,
    next: AtomicU32,
}

impl Reload {
    const fn empty() -> Self {
        Self {
            transfer: AtomicU32::new(0),
            source_end: AtomicU32::new(0),
            destination_end: AtomicU32::new(0),
            next: AtomicU32::new(0),
        }
    }

    fn address(&self) -> u32 {
        core::ptr::from_ref(self) as u32
    }

    fn link_to_itself(&self, transfer: u32, source_end: u32, destination_end: u32) {
        self.transfer.store(transfer, Ordering::Relaxed);
        self.source_end.store(source_end, Ordering::Relaxed);
        self.destination_end
            .store(destination_end, Ordering::Relaxed);
        self.next.store(self.address(), Ordering::Relaxed);
    }
}

static RELOAD_TO_FC0: Reload = Reload::empty();
static RELOAD_TO_FC2: Reload = Reload::empty();
static RELOAD_FROM_FC6: Reload = Reload::empty();
static RELOAD_FROM_FC7: Reload = Reload::empty();

const FACTORY_HEAD_TRANSFER_WORD: u32 = 0;

#[derive(Clone, Copy)]
pub enum AudioChannel {
    TransmitToFc0,
    TransmitToFc2,
    ReceiveFromFc6,
    ReceiveFromFc7,
}

enum Direction {
    Transmit(&'static DmaBuffer, Register),
    Receive(Register, &'static DmaBuffer),
}

impl AudioChannel {
    const fn number(self) -> usize {
        match self {
            AudioChannel::TransmitToFc0 => DMA0_FC0_TRANSMIT_CHANNEL_NUMBER,
            AudioChannel::TransmitToFc2 => DMA0_FC2_TRANSMIT_CHANNEL_NUMBER,
            AudioChannel::ReceiveFromFc6 => DMA0_FC6_RECEIVE_CHANNEL_NUMBER,
            AudioChannel::ReceiveFromFc7 => DMA0_FC7_RECEIVE_CHANNEL_NUMBER,
        }
    }

    const fn bit(self) -> u32 {
        match self {
            AudioChannel::TransmitToFc0 => DMA0_FC0_TRANSMIT_CHANNEL,
            AudioChannel::TransmitToFc2 => DMA0_FC2_TRANSMIT_CHANNEL,
            AudioChannel::ReceiveFromFc6 => DMA0_FC6_RECEIVE_CHANNEL,
            AudioChannel::ReceiveFromFc7 => DMA0_FC7_RECEIVE_CHANNEL,
        }
    }

    fn direction(self) -> Direction {
        match self {
            AudioChannel::TransmitToFc0 => Direction::Transmit(&AUDIO.mirrored, I2S0_FIFOWR),
            AudioChannel::TransmitToFc2 => Direction::Transmit(&AUDIO.played, I2S2_FIFOWR),
            AudioChannel::ReceiveFromFc6 => Direction::Receive(I2S6_FIFORD, &AUDIO.received),
            AudioChannel::ReceiveFromFc7 => {
                Direction::Receive(I2S7_FIFORD, &RECEIVE_FROM_FC7_DRAINED)
            }
        }
    }

    fn reload(self) -> &'static Reload {
        match self {
            AudioChannel::TransmitToFc0 => &RELOAD_TO_FC0,
            AudioChannel::TransmitToFc2 => &RELOAD_TO_FC2,
            AudioChannel::ReceiveFromFc6 => &RELOAD_FROM_FC6,
            AudioChannel::ReceiveFromFc7 => &RELOAD_FROM_FC7,
        }
    }

    fn transfer_and_ends(self) -> (u32, u32, u32) {
        match self.direction() {
            Direction::Transmit(buffer, fifo) => (
                DMA0_CHANNEL_XFERCFG_FACTORY_TRANSMIT_96_FRAMES,
                buffer.last_word_address() as u32,
                fifo.address(),
            ),
            Direction::Receive(fifo, buffer) => (
                DMA0_CHANNEL_XFERCFG_FACTORY_RECEIVE_96_FRAMES,
                fifo.address(),
                buffer.last_word_address() as u32,
            ),
        }
    }

    fn xfercfg(self) -> Register {
        DMA0_CHANNEL_XFERCFG.bytes_after(self.number() * DMA0_CHANNEL_XFERCFG_INDEX_STEP)
    }
}

pub fn start_controller() {
    clock_gates::enable(&[BusClock::Dma0]);
    dma::enable_controller();
}

pub fn start_channel(channel: AudioChannel) {
    let (transfer, source_end, destination_end) = channel.transfer_and_ends();
    let reload = channel.reload();
    reload.link_to_itself(
        transfer | DMA0_CHANNEL_XFERCFG_SWTRIG,
        source_end,
        destination_end,
    );
    dma::set_head(
        channel.number(),
        Descriptor {
            transfer: FACTORY_HEAD_TRANSFER_WORD,
            source_end,
            destination_end,
            next: reload.address(),
        },
    );
    with_the_input_mux_clock(|| INPUTMUX_DMA0_REQ_ENA_SET.write(channel.bit()));
    let number = channel.number();
    DMA0_CHANNEL_CFG
        .bytes_after(number * DMA0_CHANNEL_CFG_INDEX_STEP)
        .write(DMA0_CHANNEL_CFG_FACTORY_PERIPHERAL_REQUEST);
    DMA0_CHANNEL_CTLSTAT
        .bytes_after(number * DMA0_CHANNEL_CTLSTAT_INDEX_STEP)
        .write(DMA0_CHANNEL_CTLSTAT_FACTORY_CLEARED);
    channel.xfercfg().write(transfer);
    DMA0_ENABLESET0.write(channel.bit());
    DMA0_SETTRIG0.write(channel.bit());
}

pub fn with_the_input_mux_clock(write: impl FnOnce()) {
    SYSCON_AHBCLKCTRLSET0.write(SYSCON_AHBCLKCTRL0_MUX);
    write();
    SYSCON_AHBCLKCTRLCLR0.write(SYSCON_AHBCLKCTRL0_MUX);
}

pub fn zero_transmit_buffers() {
    AUDIO.zero_transmit_buffers();
}

#[inline(always)]
pub fn receive_reading() -> XferCount {
    XferCount::from_xfercfg(AudioChannel::ReceiveFromFc6.xfercfg().read())
}

pub fn fill_at_start_of_frame() -> DmaFill {
    AUDIO.cursor.fill_at(receive_reading().frame_in_progress())
}

pub fn ticks_served() -> u16 {
    AUDIO.cursor.ticks_served()
}
