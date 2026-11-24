mod audio_dma;
mod audio_interrupts;
mod clock_gates;
mod clocks;
mod console;
mod converter_bus;
mod dma;
mod dwt;
mod elapsed;
mod executor;
mod factory;
mod fault;
mod fault_policy;
mod halt;
mod hand_over;
mod handovers;
mod i2s;
mod image;
#[cfg(image_plant)]
mod image_plants;
mod interrupts;
mod knob_adc;
mod lcd_link;
mod map;
mod memory_fence;
mod mmio;
mod nvic;
mod panel;
mod pins;
#[cfg(any(plant = "oversize_image", plant = "stack_squeeze"))]
mod planted;
mod polled_bus;
mod power;
#[cfg(feature = "sound2")]
mod presets;
mod thread;
mod tick;
mod tick_timer;
mod transport;
mod usb;
mod usb_connection;
mod usb_controller;
mod watchdog;

use console::Console;
use cortex_m_rt::entry;
use device::boot::BOOT_RATE;
use device::console::ConsoleRings;
use dsp::{CoefficientExchange, DspModeSwitch, Meters};
use executor::Executor;
use interrupts::Startup;
use stream::{FrameRing, Playback};
use thread::{ThreadContext, ThreadPlayback};
use tick::TickContext;
use usb::UsbContext;

static DSP_MODE: DspModeSwitch = DspModeSwitch::new();

#[entry]
fn main() -> ! {
    static mut PLAYBACK: Playback = Playback::new();
    static mut CAPTURE: FrameRing = FrameRing::new();
    static mut METERS: Meters = Meters::new();
    static mut COEFFICIENTS: CoefficientExchange = CoefficientExchange::flat_at(BOOT_RATE);
    static mut CONSOLE: ConsoleRings = ConsoleRings::new();

    let previous_boot = fault::start_the_breadcrumb();

    #[cfg(any(plant = "oversize_image", plant = "stack_squeeze"))]
    planted::keep();
    #[cfg(image_plant)]
    image_plants::keep();

    let (Some(mut startup), Some(mut executor)) = (Startup::claim(), Executor::claim()) else {
        halt::forever()
    };
    #[cfg(feature = "sound3")]
    factory::read_flash_once();
    let serial = factory::serial_number();
    let (playback_usb, playback_tick, playback_counters, playback_gate) = PLAYBACK.split();
    let (capture_writer, capture_reader) = CAPTURE.split();
    let (meter_tick, meter_reader) = METERS.split();
    let (coefficient_publisher, coefficient_tick) = COEFFICIENTS.split();
    let (console_usb, console_thread) = CONSOLE.split();
    dwt::start_the_cycle_counter();

    startup.install_tick(TickContext::new(
        playback_tick,
        capture_writer,
        coefficient_tick,
        meter_tick,
        &DSP_MODE,
    ));
    startup.install_usb(UsbContext::new(
        playback_usb,
        capture_reader,
        serial,
        console_usb,
    ));
    let mut thread = ThreadContext::start(
        coefficient_publisher,
        meter_reader,
        ThreadPlayback {
            counters: playback_counters,
            gate: playback_gate,
        },
        &DSP_MODE,
        Console::new(console_thread, serial, previous_boot),
        &mut executor,
    );
    startup.let_interrupts_run();

    loop {
        thread.control_pass(&mut executor);
    }
}
