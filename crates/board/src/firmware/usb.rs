use core::sync::atomic::{AtomicBool, Ordering};

use super::audio_dma;
use super::clock_gates::{self, BusClock};
use super::halt;
use super::handovers::{CLOCK, RATE};
use super::map::USB1_RAM_FIRST_BYTE;
use super::usb_controller::usb_device_interrupt;

use device::boot::BOOT_RATE;
use device::console::ConsoleUsb;
use device::usb::answer::Descriptors;
use device::usb::dispatch::Streams;
use device::usb::endpoint::{DmaPositions, USB_RAM_BYTES, UsbRam};
use device::usb::service::Service;
use device::usb::{BENCH_DEVICE_DESCRIPTOR, BENCH_STRINGS, SerialNumber};
use dsp::RateFollower;
use stream::{CaptureUsb, Frames, PlaybackEndpoint, PlaybackUsb, RingReader, TxDmaPosition};

// WHY: USB RAM is 16 KB at 0x4010_0000 to 0x4010_3FFF and is the only memory USB1 can write, UM11126 section 44.1
const _: () = assert!(size_of::<UsbRam>() == USB_RAM_BYTES);

static USB_RAM_CLAIMED: AtomicBool = AtomicBool::new(false);

fn claim_usb_ram() -> Option<&'static mut UsbRam> {
    (!USB_RAM_CLAIMED.swap(true, Ordering::AcqRel))
        .then(|| unsafe { &mut *(USB1_RAM_FIRST_BYTE as usize as *mut UsbRam) })
}

#[repr(C)]
pub struct UsbContext {
    service: Service<'static>,
    streams_follow: RateFollower,
    console: ConsoleUsb<'static>,
}

impl UsbContext {
    pub fn new(
        playback: PlaybackUsb<'static>,
        capture: RingReader<'static>,
        serial: SerialNumber,
        console: ConsoleUsb<'static>,
    ) -> Self {
        let Some(ram) = claim_usb_ram() else {
            halt::forever()
        };
        let (buffers, _) = ram.split();
        Self {
            service: Service::new(
                buffers,
                Descriptors {
                    device: &BENCH_DEVICE_DESCRIPTOR,
                    strings: &BENCH_STRINGS,
                    serial,
                },
                Streams {
                    playback: PlaybackEndpoint::new(playback, BOOT_RATE),
                    capture: CaptureUsb::new(capture, BOOT_RATE),
                },
            ),
            streams_follow: RateFollower::of(&RATE),
            console,
        }
    }

    pub fn service(&mut self) {
        if let Some(rate) = self.streams_follow.changed(&RATE) {
            self.service.reset_streams_for_rate(rate);
        }
        usb_device_interrupt(
            &mut self.service,
            &CLOCK.latest(),
            &dma_positions(),
            &mut self.console,
        );
    }
}

fn dma_positions() -> DmaPositions {
    // WHY: DMA0 stays unclocked until the LCD or audio start, and an unclocked read may stall or fault
    if !clock_gates::is_on(BusClock::Dma0) {
        return DmaPositions {
            playback: TxDmaPosition {
                frames_queued: Frames::ZERO,
            },
            capture: Frames::ZERO,
        };
    }
    let fill = audio_dma::fill_at_start_of_frame();
    DmaPositions {
        playback: fill.playback,
        capture: fill.capture,
    }
}
