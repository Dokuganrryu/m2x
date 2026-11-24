use device::console::PowerFacts;

use super::map::{
    PMC_DCDC0, PMC_DCDC1, PMC_LDOPMU, SCB_CCR, SCB_CCR_BFHFNMIGN, SCB_CFSR, SCB_CFSR_BFARVALID,
    SCB_CFSR_PRECISERR,
};
use super::mmio::{
    Register, data_synchronization_barrier, instruction_synchronization_barrier, with_faults_masked,
};

const PRECISE_DATA_BUS_ERROR: u32 = SCB_CFSR_PRECISERR | SCB_CFSR_BFARVALID;

// WHY: PMC writes from Non-secure state take effect on this part, so the PMC answers there and has no AHBCLKCTRL gate; Armv8-M, with FAULTMASK set and CCR.BFHFNMIGN set a precise bus fault is ignored (BFHFNMINS=1), so a Secure-only word reads as unreadable instead of faulting; nothing in the PMC is written
#[inline(never)]
fn read_masking_bus_faults(register: Register) -> Option<u32> {
    with_faults_masked(|| {
        let configuration = SCB_CCR.read();
        SCB_CCR.write(configuration | SCB_CCR_BFHFNMIGN);
        SCB_CFSR.write(PRECISE_DATA_BUS_ERROR);
        data_synchronization_barrier();
        instruction_synchronization_barrier();
        let value = register.read();
        data_synchronization_barrier();
        let faulted = SCB_CFSR.read() & SCB_CFSR_PRECISERR != 0;
        SCB_CFSR.write(PRECISE_DATA_BUS_ERROR);
        SCB_CCR.write(configuration);
        data_synchronization_barrier();
        instruction_synchronization_barrier();
        (!faulted).then_some(value)
    })
}

pub fn facts() -> PowerFacts {
    PowerFacts {
        dcdc0: read_masking_bus_faults(PMC_DCDC0),
        dcdc1: read_masking_bus_faults(PMC_DCDC1),
        ldopmu: read_masking_bus_faults(PMC_LDOPMU),
    }
}

#[cfg(feature = "sound3")]
pub fn pmc_words() -> Option<device::core_clock::PmcWords> {
    Some(device::core_clock::PmcWords {
        dcdc0: read_masking_bus_faults(PMC_DCDC0)?,
        dcdc1: read_masking_bus_faults(PMC_DCDC1)?,
        ldopmu: read_masking_bus_faults(PMC_LDOPMU)?,
    })
}

// WHY: DCDC1 and LDOPMU only; each register is written only when its planned word differs, VOUT last as NXP raises it last, and the clock is raised only if every planned field reads back at least as high
#[cfg(feature = "sound3")]
pub fn raise_voltage_for_150_mhz(
    nmpa: device::core_clock::Nmpa,
) -> Option<device::core_clock::VoltagePlan> {
    use device::core_clock::{plan_for_150_mhz, voltage_reached};
    let now = pmc_words()?;
    let plan = plan_for_150_mhz(nmpa, now);
    record_plan(plan);
    for (register, before, planned) in [
        (PMC_LDOPMU, now.ldopmu, plan.writes.ldopmu),
        (PMC_DCDC1, now.dcdc1, plan.writes.dcdc1),
        (PMC_DCDC0, now.dcdc0, plan.writes.dcdc0),
    ] {
        if planned != before {
            register.write(planned);
            data_synchronization_barrier();
        }
    }
    voltage_reached(&plan, pmc_words()).then_some(plan)
}

#[cfg(feature = "sound3")]
mod planned {
    use core::sync::atomic::{AtomicU32, Ordering};

    use device::core_clock::{Corner, VoltagePlan, VoltageSource};

    static PLAN: AtomicU32 = AtomicU32::new(0);

    pub fn record(plan: VoltagePlan) {
        let corner = match plan.corner {
            Corner::Slow => 1,
            Corner::Nominal => 2,
            Corner::Fast => 3,
        };
        let source = match plan.source {
            VoltageSource::Nmpa => 1,
            VoltageSource::NmpaInvalid => 2,
            VoltageSource::NmpaUnreadable => 3,
        };
        PLAN.store(
            plan.millivolts << 8 | source << 4 | corner,
            Ordering::Relaxed,
        );
    }

    pub fn recorded() -> Option<(Corner, VoltageSource, u32)> {
        let word = PLAN.load(Ordering::Relaxed);
        let corner = match word & 0xF {
            1 => Corner::Slow,
            2 => Corner::Nominal,
            3 => Corner::Fast,
            _ => return None,
        };
        let source = match (word >> 4) & 0xF {
            1 => VoltageSource::Nmpa,
            2 => VoltageSource::NmpaInvalid,
            _ => VoltageSource::NmpaUnreadable,
        };
        Some((corner, source, word >> 8))
    }
}

#[cfg(feature = "sound3")]
use planned::record as record_plan;
#[cfg(feature = "sound3")]
pub use planned::recorded as planned_voltage;
