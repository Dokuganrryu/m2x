use device::phantom::Milliseconds;

use super::map::{
    SYST_CSR, SYST_CSR_CLKSOURCE, SYST_CSR_COUNTFLAG, SYST_CSR_ENABLE, SYST_CVR, SYST_RVR,
    SYST_RVR_FACTORY_ONE_MILLISECOND,
};

const ANY_VALUE_CLEARS_THE_COUNT: u32 = 0;

pub struct Millisecond(());

impl Millisecond {
    pub fn start() -> Self {
        SYST_RVR.write(SYST_RVR_FACTORY_ONE_MILLISECOND);
        SYST_CVR.write(ANY_VALUE_CLEARS_THE_COUNT);
        SYST_CSR.write(SYST_CSR_ENABLE | SYST_CSR_CLKSOURCE);
        Self(())
    }

    pub fn passed(&mut self) -> Option<Milliseconds> {
        take_elapsed_millisecond().then_some(Milliseconds(1))
    }
}

pub fn take_elapsed_millisecond() -> bool {
    SYST_CSR.read() & SYST_CSR_COUNTFLAG != 0
}
