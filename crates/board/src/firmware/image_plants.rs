use core::hint::black_box;

#[cfg(any(
    image_plant = "literal_pool_flash_controller",
    image_plant = "movw_movt_flash_controller"
))]
#[inline(never)]
fn name_the_controller_block() {
    const FLASH_CONTROLLER_BASE: u32 = 0x4003_4000;
    black_box(FLASH_CONTROLLER_BASE);
}

#[cfg(image_plant = "flash_driver_symbol")]
mod drivers {
    pub mod flash {
        pub struct FlashGordon;

        impl FlashGordon {
            #[inline(never)]
            pub fn erase_page(page: u32) -> u32 {
                core::hint::black_box(page)
            }
        }
    }
}

#[cfg(image_plant = "factory_read_outside_accessor")]
#[inline(never)]
fn name_the_factory_magic_outside_the_accessor() {
    const FACTORY_MAGIC_ADDRESS: u32 = 0x0003_8200;
    black_box(FACTORY_MAGIC_ADDRESS);
}

#[cfg(image_plant = "division_in_tick")]
#[inline(never)]
pub fn divide_in_tick() {
    const DIVISOR: core::num::NonZeroU32 = match core::num::NonZeroU32::new(3) {
        Some(divisor) => divisor,
        None => panic!("three is not zero"),
    };
    black_box(black_box(u32::MAX) / black_box(DIVISOR));
}

#[cfg(image_plant = "boot_request_register")]
#[inline(never)]
fn name_the_boot_request_register() {
    const AOREG1: u32 = 0x4002_0084;
    black_box(AOREG1);
}

#[cfg(image_plant = "debug_mailbox")]
#[inline(never)]
fn name_the_debug_mailbox() {
    const DEBUG_MAILBOX_BASE: u32 = 0x5009_C000;
    black_box(DEBUG_MAILBOX_BASE);
}

pub fn keep() {
    #[cfg(any(
        image_plant = "literal_pool_flash_controller",
        image_plant = "movw_movt_flash_controller"
    ))]
    name_the_controller_block();
    #[cfg(image_plant = "flash_driver_symbol")]
    black_box(drivers::flash::FlashGordon::erase_page(black_box(0)));
    #[cfg(image_plant = "factory_read_outside_accessor")]
    name_the_factory_magic_outside_the_accessor();
    #[cfg(image_plant = "boot_request_register")]
    name_the_boot_request_register();
    #[cfg(image_plant = "debug_mailbox")]
    name_the_debug_mailbox();
    black_box(());
}
