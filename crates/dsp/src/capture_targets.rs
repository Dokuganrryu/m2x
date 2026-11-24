use core::sync::atomic::{AtomicI32, Ordering};

use crate::gain::Gain;

#[derive(Debug)]
pub struct CaptureGainTargets {
    inputs: [AtomicI32; 2],
}

impl CaptureGainTargets {
    pub const fn new() -> Self {
        Self {
            inputs: [
                AtomicI32::new(Gain::UNITY.to_q5_27()),
                AtomicI32::new(Gain::UNITY.to_q5_27()),
            ],
        }
    }

    pub fn publish(&self, gains: [Gain; 2]) {
        for (input, gain) in self.inputs.iter().zip(gains) {
            input.store(gain.to_q5_27(), Ordering::Relaxed);
        }
    }

    pub fn latest(&self) -> [Gain; 2] {
        self.inputs
            .each_ref()
            .map(|input| Gain::from_q5_27(input.load(Ordering::Relaxed)))
    }
}

impl Default for CaptureGainTargets {
    fn default() -> Self {
        Self::new()
    }
}
