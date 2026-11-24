use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering, compiler_fence};

use super::dwt;
use super::hand_over;
use super::handovers::{TICK_CYCLES, USB_CYCLES};
use super::map::NVIC_USB1_IRQ;
use super::tick::TickContext;
use super::usb::UsbContext;

pub(super) type Handler = unsafe extern "C" fn();

// WHY: lpc55-pac 0.5.0 with the rt feature lists 60 device interrupts
pub(super) const DEVICE_INTERRUPTS: usize = 60;
// WHY: The audio tick interrupt is CTIMER0, IRQ10, UM11126 interrupt table
pub(super) const CTIMER0_IRQ: usize = 10;
const USB1_IRQ: usize = NVIC_USB1_IRQ;

struct InTick(());

struct InUsbInterrupt(());

struct OwnedBy<Context, T> {
    slot: UnsafeCell<MaybeUninit<T>>,
    filled: AtomicBool,
    owner: PhantomData<Context>,
}

unsafe impl<Context, T: Send> Sync for OwnedBy<Context, T> {}

impl<Context, T> OwnedBy<Context, T> {
    const fn empty() -> Self {
        Self {
            slot: UnsafeCell::new(MaybeUninit::uninit()),
            filled: AtomicBool::new(false),
            owner: PhantomData,
        }
    }

    fn fill(&self, value: T, _before_interrupts: &mut Startup) {
        if self.filled.swap(true, Ordering::Relaxed) {
            return;
        }
        unsafe { (*self.slot.get()).write(value) };
    }

    fn reach<'a>(&'a self, _owner: &'a mut Context) -> Option<&'a mut T> {
        self.filled
            .load(Ordering::Relaxed)
            .then(|| unsafe { (*self.slot.get()).assume_init_mut() })
    }
}

static TICK: OwnedBy<InTick, TickContext> = OwnedBy::empty();
static USB: OwnedBy<InUsbInterrupt, UsbContext> = OwnedBy::empty();
static STARTUP_CLAIMED: AtomicBool = AtomicBool::new(false);
static USB_INTERRUPTS_SERVED: AtomicU16 = AtomicU16::new(0);

pub fn usb_interrupts_served() -> u16 {
    USB_INTERRUPTS_SERVED.load(Ordering::Acquire)
}

pub struct Startup(());

impl Startup {
    pub fn claim() -> Option<Self> {
        (!STARTUP_CLAIMED.swap(true, Ordering::AcqRel)).then_some(Startup(()))
    }

    pub fn install_tick(&mut self, context: TickContext) {
        TICK.fill(context, self);
    }

    pub fn install_usb(&mut self, context: UsbContext) {
        USB.fill(context, self);
    }

    pub fn let_interrupts_run(self) {
        compiler_fence(Ordering::Release);
        hand_over::serve_interrupts_from_the_running_table();
    }
}

#[unsafe(no_mangle)]
extern "C" fn audio_tick_interrupt() {
    let entry = dwt::cycles();
    if let Some(tick) = TICK.reach(&mut InTick(())) {
        tick.run();
    }
    TICK_CYCLES.record_span(dwt::span(entry, dwt::cycles()));
}

#[unsafe(no_mangle)]
extern "C" fn usb_interrupt() {
    let entry = dwt::cycles();
    if let Some(usb) = USB.reach(&mut InUsbInterrupt(())) {
        usb.service();
        USB_INTERRUPTS_SERVED.fetch_add(1, Ordering::Release);
    }
    USB_CYCLES.record_span(dwt::span(entry, dwt::cycles()));
}

unsafe extern "C" {
    fn DefaultHandler();
}

pub(super) const fn device_interrupts() -> [Handler; DEVICE_INTERRUPTS] {
    let mut table: [Handler; DEVICE_INTERRUPTS] = [DefaultHandler; DEVICE_INTERRUPTS];
    table[CTIMER0_IRQ] = audio_tick_interrupt;
    table[USB1_IRQ] = usb_interrupt;
    table
}

#[unsafe(link_section = ".vector_table.interrupts")]
#[unsafe(no_mangle)]
static __INTERRUPTS: [Handler; DEVICE_INTERRUPTS] = hand_over::hand_over_handlers();
