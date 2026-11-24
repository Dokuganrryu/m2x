use crate::boot::VolumeTargets;
use crate::dac::Attenuation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VolumeSlew(VolumeTargets);

impl VolumeSlew {
    pub(crate) const fn from_ramp_end(levels: VolumeTargets) -> Self {
        Self(levels)
    }

    pub(crate) const fn levels(self) -> VolumeTargets {
        self.0
    }

    pub(crate) const fn toward(self, wanted: VolumeTargets) -> Self {
        Self(VolumeTargets {
            main: one_step_louder_at_most(self.0.main, wanted.main),
            headphone: one_step_louder_at_most(self.0.headphone, wanted.headphone),
        })
    }
}

const fn one_step_louder_at_most(now: Attenuation, wanted: Attenuation) -> Attenuation {
    let (now, wanted) = (now.half_decibel_steps(), wanted.half_decibel_steps());
    if wanted < now {
        Attenuation::from_half_decibel_steps(now - 1)
    } else {
        Attenuation::from_half_decibel_steps(wanted)
    }
}
