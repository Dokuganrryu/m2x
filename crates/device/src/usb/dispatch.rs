#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use core::sync::atomic::AtomicU32;

use stream::{AlternateSetting, CaptureUsb, PlaybackEndpoint};

use super::audio_function::{AUDIO_SLOT_BYTES, STREAM_PACKET};
use super::configuration::BusSpeed;
use super::endpoint::{
    BufferSlot, Buffers, ByteCount, Commands, DmaPositions, InEntry, InSlot, OutSlot,
};
use super::function::AudioAndSerial;

pub const PLAYBACK_PACKET_FRAMES: usize =
    STREAM_PACKET.value() as usize / AUDIO_SLOT_BYTES as usize;

const STREAM_ROOM: ByteCount = ByteCount::new(STREAM_PACKET.value());

pub trait SerialPipe {
    fn has_room(&self) -> bool;
    fn has_data(&self) -> bool;
    fn received(&mut self, words: &[AtomicU32], length: usize);
    fn send_into(&mut self, words: &[AtomicU32]) -> usize;
}

pub struct Streams<'a> {
    pub playback: PlaybackEndpoint<'a>,
    pub capture: CaptureUsb<'a>,
}

pub(crate) struct DataEndpoints<'a> {
    playback_first: OutSlot<'a>,
    playback_second: OutSlot<'a>,
    feedback: InSlot<'a>,
    capture: [InSlot<'a>; 2],
    serial_out: OutSlot<'a>,
    serial_in: InSlot<'a>,
}

const fn serial_room(speed: BusSpeed) -> ByteCount {
    ByteCount::new(speed.bulk_packet().value())
}

const fn plays(audio: Option<AudioAndSerial>) -> bool {
    match audio {
        Some(audio) => matches!(audio.playback(), AlternateSetting::One),
        None => false,
    }
}

const fn captures(audio: Option<AudioAndSerial>) -> bool {
    match audio {
        Some(audio) => matches!(audio.capture(), AlternateSetting::One),
        None => false,
    }
}

impl<'a> DataEndpoints<'a> {
    pub(crate) fn new(buffers: Buffers<'a>) -> (Self, OutSlot<'a>, InSlot<'a>) {
        let Buffers {
            control_out,
            control_in,
            playback: [playback_first, playback_second],
            feedback,
            capture: [capture_first, capture_second],
            serial_out,
            serial_in,
        } = buffers;
        (
            Self {
                playback_first: OutSlot::Held(playback_first),
                playback_second: OutSlot::Held(playback_second),
                feedback: InSlot::Held(feedback),
                capture: [InSlot::Held(capture_first), InSlot::Held(capture_second)],
                serial_out: OutSlot::Held(serial_out),
                serial_in: InSlot::Held(serial_in),
            },
            OutSlot::Held(control_out),
            InSlot::Held(control_in),
        )
    }

    #[inline(never)]
    pub(crate) fn start_of_frame(
        &mut self,
        positions: DmaPositions,
        audio: Option<AudioAndSerial>,
        serial: Option<BusSpeed>,
        streams: &mut Streams<'_>,
        pipe: &mut impl SerialPipe,
        commands: &mut Commands,
    ) {
        if plays(audio) {
            let value = streams.playback.on_sof(positions.playback);
            self.feedback.send_if_held(
                |buffer| Some(buffer.fill_words([value.samples_per_microframe_16_16()])),
                commands,
            );
        }
        if captures(audio) {
            for slot in &mut self.capture {
                slot.send_if_held(
                    |buffer| {
                        streams.capture.send_packet(positions.capture, |frames| {
                            buffer.fill_words_in_place(|words| {
                                frames.store_as_wire_words(words).count() as usize * 2
                            })
                        })
                    },
                    commands,
                );
            }
        }
        if let Some(speed) = serial {
            if matches!(self.serial_out, OutSlot::Held(_)) && pipe.has_room() {
                self.serial_out.arm(serial_room(speed), commands);
            }
            if matches!(self.serial_in, InSlot::Held(_)) && pipe.has_data() {
                self.serial_in.send_if_held(
                    |buffer| {
                        let written = buffer.fill_bytes_in_place(|words| pipe.send_into(words));
                        (written != ByteCount::ZERO).then_some(written)
                    },
                    commands,
                );
            }
        }
    }

    #[inline(never)]
    pub(crate) fn playback_completed(
        &mut self,
        slot: BufferSlot,
        untransferred: ByteCount,
        audio: Option<AudioAndSerial>,
        streams: &mut Streams<'_>,
        commands: &mut Commands,
    ) {
        let buffer = match slot {
            BufferSlot::First => &mut self.playback_first,
            BufferSlot::Second => &mut self.playback_second,
        };
        buffer.complete(untransferred, |received| {
            streams.playback.push_wire_words(received.whole_words());
        });
        if plays(audio) {
            buffer.arm(STREAM_ROOM, commands);
        }
    }

    #[inline(never)]
    pub(crate) fn serial_received(
        &mut self,
        untransferred: ByteCount,
        serial: Option<BusSpeed>,
        pipe: &mut impl SerialPipe,
        commands: &mut Commands,
    ) {
        self.serial_out.complete(untransferred, |received| {
            pipe.received(received.words(), received.length().value() as usize);
        });
        if let Some(speed) = serial
            && pipe.has_room()
        {
            self.serial_out.arm(serial_room(speed), commands);
        }
    }

    pub(crate) fn in_completed(&mut self, entry: InEntry) {
        match entry {
            InEntry::Feedback => self.feedback.complete(),
            InEntry::Capture(BufferSlot::First) => self.capture[0].complete(),
            InEntry::Capture(BufferSlot::Second) => self.capture[1].complete(),
            InEntry::Serial => self.serial_in.complete(),
            InEntry::Control => false,
        };
    }

    pub(crate) fn playback_selected(&mut self, setting: AlternateSetting, commands: &mut Commands) {
        match setting {
            AlternateSetting::One => {
                self.playback_first.arm(STREAM_ROOM, commands);
                self.playback_second.arm(STREAM_ROOM, commands);
            }
            AlternateSetting::Zero => {
                self.playback_first.retire(commands);
                self.playback_second.retire(commands);
                self.feedback.retire(commands);
            }
        }
    }

    pub(crate) fn capture_selected(&mut self, setting: AlternateSetting, commands: &mut Commands) {
        match setting {
            AlternateSetting::One => {}
            AlternateSetting::Zero => {
                for slot in &mut self.capture {
                    slot.retire(commands);
                }
            }
        }
    }

    pub(crate) fn serial_opened(&mut self, speed: BusSpeed, commands: &mut Commands) {
        self.serial_out.arm(serial_room(speed), commands);
    }

    pub(crate) fn serial_closed(&mut self, commands: &mut Commands) {
        self.serial_out.retire(commands);
        self.serial_in.retire(commands);
    }

    pub(crate) fn reclaim_after_reset(&mut self) {
        self.playback_first.reclaim();
        self.playback_second.reclaim();
        self.feedback.reclaim();
        for slot in &mut self.capture {
            slot.reclaim();
        }
        self.serial_out.reclaim();
        self.serial_in.reclaim();
    }
}
