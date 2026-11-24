#[derive(Clone, Copy)]
pub enum FaultPolicy {
    BringUp,
    Release,
}

pub const FAULT_POLICY: FaultPolicy = if cfg!(feature = "release-policy") {
    FaultPolicy::Release
} else {
    FaultPolicy::BringUp
};
