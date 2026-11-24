#![no_std]
#![no_main]
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

static mut LEVEL: u32 = 0;
static mut WIDE: u64 = 0;
static mut TABLE: [u32; 4] = [0; 4];
static mut HANDLER: fn() = idle;

fn idle() {}

#[inline(never)]
fn mix_block(gain: u32) -> u32 {
    unsafe {
        let level = read_volatile(&raw const LEVEL);
        #[cfg(plant = "udiv_in_tick")]
        let level = level.checked_div(gain).unwrap_or(0);
        #[cfg(plant = "aeabi_div_in_tick")]
        let level = (read_volatile(&raw const WIDE).checked_div(gain as u64).unwrap_or(0)) as u32;
        #[cfg(plant = "vfp_in_tick")]
        let level = (level as f32 * 0.5) as u32;
        #[cfg(plant = "panic_in_tick")]
        let level = TABLE[level as usize];
        #[cfg(plant = "masking_in_tick")]
        core::arch::asm!("cpsid i");
        #[cfg(plant = "critical_section_in_tick")]
        _critical_section_1_0_acquire();
        #[cfg(plant = "libm_in_tick")]
        let level = libm::sqrt_bits(level);
        #[cfg(plant = "flash_controller_interleaved")]
        let level = level ^ read_volatile(0x4003_4020 as *const u32);
        #[cfg(plant = "fmt_in_tick")]
        let _ = core::fmt::write(&mut Sink, format_args!("{}", level));
        #[cfg(plant = "indirect_call_in_tick")]
        read_volatile(&raw const HANDLER)();
        write_volatile(&raw mut LEVEL, level.wrapping_mul(gain) >> 3);
        level
    }
}

#[cfg(plant = "critical_section_in_tick")]
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn _critical_section_1_0_acquire() {
    unsafe { write_volatile(&raw mut LEVEL, 1) }
}

#[cfg(plant = "libm_in_tick")]
mod libm {
    #[inline(never)]
    pub fn sqrt_bits(x: u32) -> u32 {
        x.isqrt()
    }
}

#[cfg(plant = "fmt_in_tick")]
struct Sink;

#[cfg(plant = "fmt_in_tick")]
impl core::fmt::Write for Sink {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        unsafe { write_volatile(&raw mut LEVEL, text.len() as u32) };
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CTIMER0() {
    mix_block(3);
}

#[inline(never)]
fn fill_ring(n: u32) {
    unsafe {
        write_volatile(&raw mut LEVEL, n.wrapping_add(7));
        #[cfg(plant = "flash_controller")]
        write_volatile(0x4003_4010 as *mut u32, n);
        #[cfg(plant = "flash_controller")]
        write_volatile(0x4003_4ff0 as *mut u32, n.wrapping_add(1));
        #[cfg(plant = "flash_controller_secure_alias")]
        write_volatile(0x5003_4008 as *mut u32, n);
        #[cfg(plant = "rom_flash_api_entry")]
        write_volatile(&raw mut LEVEL, 0xFEED_0001);
        #[cfg(plant = "rom_api_tree")]
        write_volatile(&raw mut LEVEL, read_volatile(0x1300_10F0 as *const u32));
        #[cfg(plant = "rom_api_tree_non_secure_alias")]
        write_volatile(&raw mut LEVEL, read_volatile(0x0300_10F0 as *const u32));
        #[cfg(plant = "rom_flash_erase_entry_version_0")]
        write_volatile(&raw mut LEVEL, 0x1300_413B);
        #[cfg(plant = "rom_flash_erase_entry_non_secure_alias")]
        write_volatile(&raw mut LEVEL, 0x0300_413B);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn USB1() {
    fill_ring(5);
    mix_block(2);
}

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn factory_serial(out: &mut [u32; 2]) {
    unsafe {
        out[0] = read_volatile(0x0003_8200 as *const u32);
        out[1] = read_volatile(0x0003_8204 as *const u32);
        #[cfg(plant = "factory_write_in_accessor")]
        write_volatile(0x0003_8200 as *mut u32, out[1]);
    }
}

#[cfg(plant = "flash_driver_symbol")]
mod lpc55_hal {
    pub mod drivers {
        pub mod flash {
            #[inline(never)]
            pub fn erase_page(page: u32) {
                unsafe { core::ptr::write_volatile(&raw mut super::super::super::LEVEL, page) }
            }
        }
    }
}

#[cfg(plant = "dead_stripped_symbol_kept")]
mod packet_sizer {
    pub struct NominalPacket;
    impl NominalPacket {
        #[inline(never)]
        pub fn at(rate: u32) -> u32 {
            rate.wrapping_mul(6)
        }
    }
}

#[cfg(plant = "dac_register_zero_writer")]
mod protocol {
    pub struct RegisterZeroWrite;
    impl RegisterZeroWrite {
        #[inline(never)]
        pub fn transaction(value: u32) -> u32 {
            value << 8
        }
    }
}

#[cfg(plant = "vfp_outside_interrupts")]
mod console {
    #[inline(never)]
    pub fn band_gain(tenths: u32) -> u32 {
        (tenths as f32 * 0.1) as u32
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    let mut serial = [0u32; 2];
    factory_serial(&mut serial);
    unsafe {
        write_volatile(&raw mut LEVEL, serial[0]);
        #[cfg(plant = "factory_record_outside_accessor")]
        write_volatile(&raw mut LEVEL, read_volatile(0x0003_8400 as *const u32));
        #[cfg(plant = "factory_magic_outside_accessor")]
        write_volatile(&raw mut LEVEL, read_volatile(0x0003_8200 as *const u32));
        #[cfg(plant = "flash_driver_symbol")]
        lpc55_hal::drivers::flash::erase_page(serial[1]);
        #[cfg(plant = "dead_stripped_symbol_kept")]
        write_volatile(&raw mut LEVEL, packet_sizer::NominalPacket::at(serial[1]));
        #[cfg(plant = "dac_register_zero_writer")]
        write_volatile(&raw mut LEVEL, protocol::RegisterZeroWrite::transaction(serial[1]));
        #[cfg(plant = "vfp_outside_interrupts")]
        write_volatile(&raw mut LEVEL, console::band_gain(serial[1]));
        write_volatile(&raw mut WIDE, 0);
        write_volatile(&raw mut HANDLER, idle);
        let _ = read_volatile(&raw const TABLE);
    }
    loop {}
}
