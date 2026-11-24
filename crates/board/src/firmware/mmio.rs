use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy)]
pub struct Register(usize);

impl Register {
    pub(super) const fn at(address: usize) -> Self {
        Self(address)
    }

    pub const fn address(self) -> u32 {
        self.0 as u32
    }

    pub const fn bytes_after(self, bytes: usize) -> Self {
        Self(self.0 + bytes)
    }

    pub fn read(self) -> u32 {
        unsafe { read_volatile(self.0 as *const u32) }
    }

    pub fn write(self, value: u32) {
        unsafe { write_volatile(self.0 as *mut u32, value) }
    }

    // WHY: Arm DDI 0553A.h B5.23 RWLWT, every load and store before a Store-Release is observed before its write
    pub fn write_release(self, value: u32) {
        unsafe { AtomicU32::from_ptr(self.0 as *mut u32) }.store(value, Ordering::Release);
    }

    pub fn set(self, bits: u32) {
        self.write(self.read() | bits);
    }
}

#[cfg(any(not(feature = "milestone4"), feature = "sound1"))]
#[derive(Clone, Copy)]
pub struct MemoryBytes(usize);

#[cfg(any(not(feature = "milestone4"), feature = "sound1"))]
impl MemoryBytes {
    pub(super) const fn at(address: usize) -> Self {
        Self(address)
    }

    #[inline(always)]
    pub fn read_word(self) -> u32 {
        unsafe { read_volatile(self.0 as *const u32) }
    }

    #[inline(always)]
    pub fn read_bytes<const COUNT: usize>(self) -> [u8; COUNT] {
        let mut bytes = [0; COUNT];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = unsafe { read_volatile((self.0 as *const u8).add(offset)) };
        }
        bytes
    }
}

pub fn data_synchronization_barrier() {
    unsafe { asm!("dsb sy", options(nostack, preserves_flags)) }
}

pub fn instruction_synchronization_barrier() {
    unsafe { asm!("isb sy", options(nostack, preserves_flags)) }
}

pub fn data_memory_barrier() {
    unsafe { asm!("dmb sy", options(nostack, preserves_flags)) }
}

pub fn mask_every_exception_but_nmi() {
    unsafe { asm!("cpsid f", options(nostack, preserves_flags)) }
}

#[inline(always)]
pub fn with_interrupts_masked<T>(work: impl FnOnce() -> T) -> T {
    let primask: u32;
    unsafe {
        asm!("mrs {}, PRIMASK", "cpsid i", out(reg) primask, options(nostack, preserves_flags));
    }
    let result = work();
    if primask & 1 == 0 {
        unsafe { asm!("cpsie i", options(nostack, preserves_flags)) }
    }
    result
}

#[inline(always)]
pub fn with_faults_masked<T>(work: impl FnOnce() -> T) -> T {
    let faultmask: u32;
    unsafe {
        asm!("mrs {}, FAULTMASK", "cpsid f", out(reg) faultmask, options(nostack, preserves_flags));
    }
    let result = work();
    if faultmask & 1 == 0 {
        unsafe { asm!("cpsie f", options(nostack, preserves_flags)) }
    }
    result
}

pub fn unmask_interrupts() {
    unsafe { asm!("cpsie i", options(nostack, preserves_flags)) }
}

macro_rules! quiesce_at_entry {
    ($writes:path $(, $first:literal)*) => {
        core::arch::global_asm!(
            ".section .text.__pre_init,\"ax\",%progbits",
            ".global __pre_init",
            ".type __pre_init,%function",
            ".thumb_func",
            "__pre_init:",
            $($first,)*
            "cpsid i",
            "ldr r0, ={writes}",
            "0:",
            "ldm r0!, {{r1, r2, r3}}",
            "cbz r1, 2f",
            "tst r1, #1",
            "bne 3f",
            "cbz r2, 1f",
            "ldr r12, [r1]",
            "and r12, r12, r2",
            "orr r3, r3, r12",
            "1:",
            "str r3, [r1]",
            "b 0b",
            "3:",
            "ldr r12, [r1, #-1]",
            "tst r12, r2",
            "bne 0b",
            "add r3, r3, r3, lsl #1",
            "add r0, r0, r3, lsl #2",
            "b 0b",
            "2:",
            "dsb sy",
            "isb sy",
            "bx lr",
            ".ltorg",
            ".size __pre_init, . - __pre_init",
            ".global SysTick",
            ".thumb_set SysTick, __pre_init",
            ".global PendSV",
            ".thumb_set PendSV, __pre_init",
            writes = sym $writes,
        );
    };
}

pub(crate) use quiesce_at_entry;
