const PLANTED_BYTES: usize = match usize::from_str_radix(env!("BOARD_PLANTED_BYTES"), 10) {
    Ok(bytes) => bytes,
    Err(_) => panic!("BOARD_PLANTED_BYTES is a decimal byte count"),
};

#[cfg(plant = "oversize_image")]
static PLANTED: [u8; PLANTED_BYTES] = [0xA5; PLANTED_BYTES];

#[cfg(plant = "stack_squeeze")]
static mut PLANTED: [u8; PLANTED_BYTES] = [0; PLANTED_BYTES];

pub fn keep() {
    core::hint::black_box(&raw const PLANTED);
}
