use device::factory::FactoryRead;
use device::usb::SerialNumber;

// WHY: An unreadable page gives the template serial, so a build that never reads flash serves that template
#[cfg(feature = "milestone4")]
pub fn serial_number() -> SerialNumber {
    FactoryRead::Unreadable.serial_number()
}

#[cfg(not(feature = "milestone4"))]
pub fn serial_number() -> SerialNumber {
    page::read_factory_page().serial_number()
}

// WHY: The factory page at 0x38200 is Non-secure and readable with a plain AHB load; the read masks a precise bus fault (an erased page) and reports Unreadable; the USB descriptor keeps the template serial, so the host always sees the same device instance
#[cfg(all(feature = "sound1", not(feature = "sound3")))]
pub fn read_on_demand() -> Option<FactoryRead> {
    Some(page::read_factory_page())
}

// WHY: LPC55S2x data sheet, a single-word flash read is allowed only at or below 100 MHz, and the 150 MHz build leaves FMCCR at nine clocks; so the factory page and the four NMPA words are read once in main at the loader's clock, every later reader takes the copy, and no flash address is read at 150 MHz
#[cfg(feature = "sound3")]
mod cache {
    use core::cell::UnsafeCell;
    use core::sync::atomic::{AtomicBool, Ordering};

    use device::core_clock::Nmpa;
    use device::factory::FactoryRead;

    struct Copies {
        factory: FactoryRead,
        nmpa: Nmpa,
    }

    struct Cell(UnsafeCell<Copies>);

    unsafe impl Sync for Cell {}

    static COPIES: Cell = Cell(UnsafeCell::new(Copies {
        factory: FactoryRead::Unreadable,
        nmpa: Nmpa::Unreadable,
    }));
    static TAKEN: AtomicBool = AtomicBool::new(false);

    // WHY: And every other access only reads, so the one write cannot race a read
    pub fn take() {
        if TAKEN.swap(true, Ordering::AcqRel) {
            return;
        }
        let factory = super::page::read_factory_page();
        let nmpa = super::page::read_nmpa();
        unsafe {
            *COPIES.0.get() = Copies { factory, nmpa };
        }
    }

    pub fn factory() -> FactoryRead {
        unsafe { (*COPIES.0.get()).factory }
    }

    pub fn nmpa() -> Nmpa {
        unsafe { (*COPIES.0.get()).nmpa }
    }
}

#[cfg(feature = "sound3")]
pub use cache::{nmpa, take as read_flash_once};

#[cfg(feature = "sound3")]
pub fn read_on_demand() -> Option<FactoryRead> {
    Some(cache::factory())
}

#[cfg(not(feature = "sound1"))]
pub fn read_on_demand() -> Option<FactoryRead> {
    None
}

#[cfg(any(not(feature = "milestone4"), feature = "sound1"))]
mod page {
    use device::factory::FactoryRead;
    use device::usb::SERIAL_NUMBER_LENGTH;

    use super::super::map::{
        FACTORY_MAGIC, FACTORY_SERIAL, MPU_CTRL, MPU_CTRL_OFF, SCB_CCR, SCB_CCR_BFHFNMIGN,
        SCB_CFSR, SCB_CFSR_BFARVALID, SCB_CFSR_PRECISERR,
    };
    use super::super::mmio::{
        data_synchronization_barrier, instruction_synchronization_barrier, with_faults_masked,
    };

    const PRECISE_DATA_BUS_ERROR: u32 = SCB_CFSR_PRECISERR | SCB_CFSR_BFARVALID;

    #[unsafe(no_mangle)]
    #[inline(never)]
    pub fn read_factory_page() -> FactoryRead {
        with_faults_masked(|| {
            let mpu = MPU_CTRL.read();
            let configuration = SCB_CCR.read();
            MPU_CTRL.write(MPU_CTRL_OFF);
            SCB_CCR.write(configuration | SCB_CCR_BFHFNMIGN);
            SCB_CFSR.write(PRECISE_DATA_BUS_ERROR);
            data_synchronization_barrier();
            instruction_synchronization_barrier();
            let magic = FACTORY_MAGIC.read_word();
            let serial: [u8; SERIAL_NUMBER_LENGTH] = FACTORY_SERIAL.read_bytes();
            data_synchronization_barrier();
            let faulted = SCB_CFSR.read() & SCB_CFSR_PRECISERR != 0;
            SCB_CFSR.write(PRECISE_DATA_BUS_ERROR);
            SCB_CCR.write(configuration);
            MPU_CTRL.write(mpu);
            data_synchronization_barrier();
            instruction_synchronization_barrier();
            if faulted {
                FactoryRead::Unreadable
            } else {
                FactoryRead::Loaded { magic, serial }
            }
        })
    }

    #[cfg(feature = "sound3")]
    #[inline(never)]
    pub fn read_nmpa() -> device::core_clock::Nmpa {
        use super::super::map::NMPA_WORDS;
        use device::core_clock::{Nmpa, NmpaWords};
        with_faults_masked(|| {
            let mpu = MPU_CTRL.read();
            let configuration = SCB_CCR.read();
            MPU_CTRL.write(MPU_CTRL_OFF);
            SCB_CCR.write(configuration | SCB_CCR_BFHFNMIGN);
            SCB_CFSR.write(PRECISE_DATA_BUS_ERROR);
            data_synchronization_barrier();
            instruction_synchronization_barrier();
            let words = NMPA_WORDS.map(|word| word.read_word());
            data_synchronization_barrier();
            let faulted = SCB_CFSR.read() & SCB_CFSR_PRECISERR != 0;
            SCB_CFSR.write(PRECISE_DATA_BUS_ERROR);
            SCB_CCR.write(configuration);
            MPU_CTRL.write(mpu);
            data_synchronization_barrier();
            instruction_synchronization_barrier();
            if faulted {
                Nmpa::Unreadable
            } else {
                Nmpa::Read(NmpaWords::from_words(words))
            }
        })
    }
}
