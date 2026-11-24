use device::boot::Image;

// WHY: Milestone7 runs every boot stage, as the full image does, on the milestone4 hand-over, clock and USB path; silent.rs holds its outputs and phantom off
pub const IMAGE: Image = if cfg!(feature = "quiesce-only") {
    Image::QuiesceOnly
} else if cfg!(feature = "milestone7") {
    Image::Full
} else if cfg!(feature = "milestone") {
    Image::Milestone
} else {
    Image::Full
};
