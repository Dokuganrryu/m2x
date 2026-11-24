#![no_std]
#![forbid(unsafe_code)]

mod digest;
mod grid;
mod run;
mod script;
mod source;

pub use digest::{Channel, TickDigest};
pub use grid::{FILTER_KINDS, design_grid_digest, five_band_preset};
pub use run::{GoldenRun, TickWords, standard_run};
pub use script::{Cue, STANDARD_SCRIPT, Script, ScriptCue};
pub use source::{FrameSource, SeededNoise, SourceFrame};
