use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m_rt::exception;
use device::breadcrumb::{At, Breadcrumb, MAGIC, Place, Weight};
use device::command::Command;
use device::fault::{FaultCode, fault_routine};
use device::lcd::panel::{INFERRED_PANEL_SCK, SpiByteCount};
use device::lcd::screen::{FAULT_WINDOW, FULL_SCREEN_CLEAR_SPI_BYTES};

use super::audio_dma;
use super::elapsed;
use super::fault_policy::{FAULT_POLICY, FaultPolicy};
use super::halt;
use super::lcd_link::LcdLink;
use super::map::{
    SCB_AIRCR, SCB_AIRCR_PRIGROUP, SCB_AIRCR_SYSRESETREQ, SCB_AIRCR_VECTKEY, SCB_CCR,
    SCB_CCR_BFHFNMIGN,
};
use super::mmio::{data_synchronization_barrier, mask_every_exception_but_nmi};
use super::pins;
use super::polled_bus;
use super::transport;
use super::watchdog;

#[unsafe(link_section = ".uninit.BREADCRUMB")]
#[unsafe(no_mangle)]
static BREADCRUMB: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

#[derive(Clone, Copy)]
enum Ending {
    Park,
    ResetTheChip,
    HaltForTheDebugger,
}

const ENDING: Ending = match FAULT_POLICY {
    FaultPolicy::BringUp => Ending::Park,
    FaultPolicy::Release if cfg!(debug_assertions) => Ending::HaltForTheDebugger,
    FaultPolicy::Release => Ending::ResetTheChip,
};

pub fn enter(code: FaultCode) -> ! {
    mask_every_exception_but_nmi();
    SCB_CCR.set(SCB_CCR_BFHFNMIGN);
    for command in fault_routine(code) {
        execute(command);
    }
    show_the_fault_screen(code);
    end()
}

// WHY: A SysTick wrap is 100,000 core cycles; each bound is the transfer time at the inferred SPI clock plus a quarter, rounded up to wraps, plus one wrap for the partial first one
const fn systick_wraps_to_send(bytes: SpiByteCount) -> u32 {
    (bytes.transfer_time(INFERRED_PANEL_SCK).0 * 5 / 4).div_ceil(1_000) + 1
}

const IN_FLIGHT_WRAPS: u32 = systick_wraps_to_send(FULL_SCREEN_CLEAR_SPI_BYTES);
const FAULT_SCREEN_WRAPS: u32 = systick_wraps_to_send(FAULT_WINDOW.transfer_bytes());

fn show_the_fault_screen(code: FaultCode) {
    let Some(mut link) = LcdLink::for_the_fatal_screen() else {
        return;
    };
    if finishes_within(&link, IN_FLIGHT_WRAPS) {
        link.send_fault(code, breadcrumb().place.stage);
        finishes_within(&link, FAULT_SCREEN_WRAPS);
    }
}

#[inline(never)]
fn finishes_within(link: &LcdLink, wraps: u32) -> bool {
    let mut left = wraps;
    while !link.finished() {
        if elapsed::take_elapsed_millisecond() {
            watchdog::feed_while_parked();
            left -= 1;
            if left == 0 {
                return false;
            }
        }
    }
    true
}

fn execute(command: Command) {
    let command = super::executor::as_built(command);
    match command {
        Command::StopTransmitDma => transport::stop_transmit_dma(),
        Command::ZeroTransmitBuffers => audio_dma::zero_transmit_buffers(),
        Command::DrivePin(pin, level) => pins::drive(pin.pin(), level),
        Command::SendAnalogWord(word) => {
            polled_bus::send_analog_word_from_any_state_and_wait_until_latched(word);
        }
        Command::RecordFault(code, at, weight) => record(code, at, weight),
        Command::WriteDac(_)
        | Command::ReadDac(_)
        | Command::ProbeAdc(_)
        | Command::WriteAdc(_)
        | Command::SwitchPll0Family(_)
        | Command::StartI2s(_)
        | Command::StopI2s
        | Command::StartDma
        | Command::StopDma
        | Command::StartTickTimer(_)
        | Command::StopTickTimer
        | Command::MuteInput(_)
        | Command::UnmuteInput(_)
        | Command::AttachUsb
        | Command::DisableInterrupts
        | Command::DetachUsb
        | Command::FeedWatchdog
        | Command::ArmWatchdog
        | Command::FenceFlashController
        | Command::ConfigureAnalogWordBus
        | Command::StartClockInput
        | Command::ReadClockInput
        | Command::StartCorePll
        | Command::SelectCorePllAsMainClock
        | Command::EnableMasterClockOutput
        | Command::ConfigureConverterBus
        | Command::WriteExpanderDirection
        | Command::ReadExpanderInputs
        | Command::WriteExpanderOutputs(_)
        | Command::StartKnobScan
        | Command::LatchPlaybackSilence => {}
    }
}

// WHY: The fresh record goes in before the magic word, so the magic never vouches for a record this boot did not write
pub fn start_the_breadcrumb() -> Option<Breadcrumb> {
    let [magic, record] = &BREADCRUMB;
    let previous = Breadcrumb::previous([
        magic.load(Ordering::Relaxed),
        record.load(Ordering::Relaxed),
    ]);
    record.store(Breadcrumb::START.pack(), Ordering::Relaxed);
    magic.store(MAGIC, Ordering::Relaxed);
    previous
}

pub fn breadcrumb() -> Breadcrumb {
    Breadcrumb::unpack(BREADCRUMB[1].load(Ordering::Relaxed))
}

#[inline(never)]
pub fn mark(place: Place) {
    update(|crumb| crumb.marked(place));
}

#[inline(never)]
pub fn record(code: FaultCode, at: At, weight: Weight) {
    update(|crumb| crumb.recorded(code, at, weight));
}

// WHY: A plain load and store, not a compare-exchange loop: only the thread and the fatal path write the word, and the fatal path never returns to the thread
fn update(rule: impl FnOnce(Breadcrumb) -> Breadcrumb) {
    let record = &BREADCRUMB[1];
    let word = record.load(Ordering::Relaxed);
    let next = rule(Breadcrumb::unpack(word)).pack();
    if next != word {
        record.store(next, Ordering::Relaxed);
    }
}

fn end() -> ! {
    match ENDING {
        Ending::Park => park(),
        Ending::HaltForTheDebugger => halt::forever(),
        Ending::ResetTheChip => {
            data_synchronization_barrier();
            SCB_AIRCR.write(
                SCB_AIRCR_VECTKEY | (SCB_AIRCR.read() & SCB_AIRCR_PRIGROUP) | SCB_AIRCR_SYSRESETREQ,
            );
            data_synchronization_barrier();
            halt::forever()
        }
    }
}

fn park() -> ! {
    loop {
        if elapsed::take_elapsed_millisecond() {
            watchdog::feed_while_parked();
        }
    }
}

#[panic_handler]
fn fault_on_panic(_: &PanicInfo) -> ! {
    enter(FaultCode::Panic)
}

#[exception(trampoline = false)]
unsafe fn HardFault() -> ! {
    enter(FaultCode::HardFault)
}

#[exception]
unsafe fn DefaultHandler(_: i16) -> ! {
    enter(FaultCode::UnexpectedException)
}
