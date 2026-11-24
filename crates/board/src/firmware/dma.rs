use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use super::map::{DMA0_CTRL, DMA0_CTRL_ENABLE, DMA0_SRAMBASE};
use super::mmio::data_synchronization_barrier;

// WHY: DMA0 has 23 channels, each with a 16-byte head entry in the table SRAMBASE points at
const CHANNELS: usize = 23;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub transfer: u32,
    pub source_end: u32,
    pub destination_end: u32,
    pub next: u32,
}

impl Descriptor {
    pub const UNUSED: Self = Self {
        transfer: 0,
        source_end: 0,
        destination_end: 0,
        next: 0,
    };

    pub fn address(&self) -> u32 {
        core::ptr::from_ref(self) as u32
    }
}

// WHY: The table is 512-byte aligned for SRAMBASE, UM11126 DMA SRAMBASE
#[repr(C, align(512))]
struct ChannelTable(UnsafeCell<MaybeUninit<[Descriptor; CHANNELS]>>);

unsafe impl Sync for ChannelTable {}

#[unsafe(link_section = ".panel_buffers.dma_table")]
static TABLE: ChannelTable = ChannelTable(UnsafeCell::new(MaybeUninit::uninit()));
static TABLE_CLEARED: AtomicBool = AtomicBool::new(false);

fn table() -> *mut Descriptor {
    TABLE.0.get().cast::<Descriptor>()
}

pub fn enable_controller() {
    if !TABLE_CLEARED.swap(true, Ordering::AcqRel) {
        unsafe { table().write_bytes(0, CHANNELS) }
    }
    data_synchronization_barrier();
    DMA0_SRAMBASE.write(table() as u32);
    DMA0_CTRL.write(DMA0_CTRL_ENABLE);
}

pub fn set_head(channel: usize, head: Descriptor) {
    if channel < CHANNELS && TABLE_CLEARED.load(Ordering::Acquire) {
        unsafe { table().add(channel).write(head) }
    }
    data_synchronization_barrier();
}
