use std::env;
use std::fs;
use std::path::PathBuf;

const NO_DEVICE_HANDLER_ALIASES: &str = "";
// WHY: Reset sits at image offset 0x200, where the loader that starts the image jumps
const ENTRY_AT_0X200: &str = "
_stext = ORIGIN(FLASH) + 0x200;
ASSERT((Reset & ~1) == ORIGIN(FLASH) + 0x200, \"board: milestone2 needs Reset at image offset 0x200, where the loader jumps\");
";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(plant, values(\"oversize_image\", \"stack_squeeze\"))");
    println!(
        "cargo::rustc-check-cfg=cfg(image_plant, values(none(), \"literal_pool_flash_controller\", \"movw_movt_flash_controller\", \"flash_driver_symbol\", \"factory_read_outside_accessor\", \"division_in_tick\", \"boot_request_register\", \"debug_mailbox\"))"
    );
    println!("cargo::rerun-if-changed=memory.x");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let mut memory = fs::read_to_string("memory.x").expect("memory.x is readable");
    if env::var_os("CARGO_FEATURE_MILESTONE2").is_some() {
        memory.push_str(ENTRY_AT_0X200);
    }
    fs::write(out.join("memory.x"), memory).expect("memory.x is written into OUT_DIR");
    fs::write(out.join("device.x"), NO_DEVICE_HANDLER_ALIASES)
        .expect("device.x is written into OUT_DIR");
    println!("cargo::rustc-link-search={}", out.display());
    println!("cargo::rustc-link-arg-bins=-Tlink.x");
}
