#![no_std]
#![forbid(unsafe_code)]

mod capture;
mod capture_machine;
mod endpoint;
mod fade;
mod feedback;
mod frames;
mod machine;
mod packet_sizer;
mod playback;
mod ring;
mod transport;

pub use capture::{CapturePacket, CaptureUsb, PacketFrames};
pub use capture_machine::{CaptureCommand, CaptureEvent, CaptureState};
pub use endpoint::PlaybackEndpoint;
pub use feedback::{FeedbackEvent, FeedbackState, FeedbackValue, Integrator};
pub use frames::Frames;
pub use machine::{PlaybackCommand, PlaybackEvent, PlaybackState};
pub use packet_sizer::{MAX_CAPTURE_PACKET_FRAMES, PacketSizer};
pub use playback::{
    AlternateSetting, FillRange, Playback, PlaybackCounterReader, PlaybackCounters, PlaybackGate,
    PlaybackTick, PlaybackUsb,
};
pub use ring::{FrameRing, RING_CAPACITY, RingReader, RingWriter};
pub use transport::{
    DMA_BUFFER_FRAMES, DmaBuffer, DmaCursor, DmaFill, DmaFrame, DmaTransport, TRANSMIT_LEAD_BLOCKS,
    TickBlocks, TxDmaPosition, XferCount, tick_counter_start,
};
