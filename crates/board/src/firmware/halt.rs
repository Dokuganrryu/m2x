pub fn forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
