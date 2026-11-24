// WHY: The image runs Non-secure; Armv8-M ARM NSACR, a Non-secure VFP instruction is a NOCP UsageFault unless the Secure side grants CP10 and CP11, and the loader is not known to; rustc platform-support thumbv8m.main-none-eabi, Cortex-M33 with DSP and no FPU is this target with `cortex-m33` and `-fpregs`, so LLVM cannot emit a VFP instruction, `f64` stays the same software routines, and cortex-m-rt no longer enables CP10 and CP11 in CPACR
pub const TARGET: &str = "thumbv8m.main-none-eabi";
pub const RUSTFLAGS: &str = "-C target-cpu=cortex-m33 -C target-feature=-fpregs";
