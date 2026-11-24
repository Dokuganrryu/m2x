#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use std::collections::{BTreeSet, HashMap, VecDeque};

use device::rate_change;
use device::usb::answer::Descriptors;
use device::usb::dispatch::{SerialPipe, Streams};
use device::usb::endpoint::{
    BufferSlot, ByteCount, Command, ControllerView, DmaPositions, Event, FrameStart, InEntry,
    ListEntry, OutEntry, PlaybackArrival, StreamCompletions, UsbRam,
};
use device::usb::request::{Address, HaltableEndpoint};
use device::usb::service::{Notice, Service};
use device::usb::{BENCH_DEVICE_DESCRIPTOR, BENCH_STRINGS, BusSpeed, FACTORY_SERIAL_TEMPLATE};
use dsp::{Frame, MAX_FRAMES, SampleRate, StreamSample};
use stream::{
    CaptureUsb, FeedbackValue, FrameRing, Frames, Playback, PlaybackEndpoint, PlaybackTick,
    RingWriter, TxDmaPosition,
};

pub const CONTROL_PACKET: usize = 64;
pub const FEEDBACK_POLL_MICROFRAMES: u64 = 8;

#[derive(Debug, PartialEq, Eq)]
pub struct Stalled;

#[derive(Default)]
pub struct Console {
    pub from_host: Vec<u8>,
    pub to_host: VecDeque<u8>,
    pub from_host_held: bool,
}

impl SerialPipe for Console {
    fn has_room(&self) -> bool {
        !self.from_host_held
    }

    fn has_data(&self) -> bool {
        !self.to_host.is_empty()
    }

    fn received(&mut self, words: &[AtomicU32], length: usize) {
        let bytes = words
            .iter()
            .flat_map(|word| word.load(Ordering::Relaxed).to_le_bytes());
        self.from_host.extend(bytes.take(length));
    }

    fn send_into(&mut self, words: &[AtomicU32]) -> usize {
        let mut sent = 0;
        for word in words
            .iter()
            .take(device::console::OUTBOUND_PACKET_BYTES.div_ceil(4))
        {
            let mut lanes = [0u8; 4];
            for lane in &mut lanes {
                if sent < device::console::OUTBOUND_PACKET_BYTES
                    && let Some(byte) = self.to_host.pop_front()
                {
                    *lane = byte;
                    sent += 1;
                }
            }
            word.store(u32::from_le_bytes(lanes), Ordering::Relaxed);
        }
        sent
    }
}

#[derive(Default)]
pub struct StreamLog {
    pub playback_frames_sent: u64,
    pub playback_packets_lost: u64,
    pub capture_packets: Vec<Vec<u8>>,
    pub capture_polls_unanswered: u64,
    pub feedback: Vec<u32>,
    pub rendered_nonsilent_frames: u64,
}

pub struct Bench<'a> {
    pub service: Service<'a>,
    controller: ControllerView<'a>,
    tick: PlaybackTick<'a>,
    capture_writer: RingWriter<'a>,
    armed: HashMap<ListEntry, ByteCount>,
    pub halted: BTreeSet<u8>,
    pub control_stalled: bool,
    pub address: Address,
    address_after_status: Option<Address>,
    pub rate: rate_change::State,
    pub console: Console,
    pub notices: Vec<Notice>,
    pub microframe: u64,
    host_phase: u64,
    next_playback: BufferSlot,
    next_capture: BufferSlot,
    capture_counter: i32,
    playback_counter: i32,
    pub log: StreamLog,
    pub completions_ride_on_the_next_start_of_frame: bool,
    completed_since_the_last_start_of_frame: StreamCompletions,
}

pub fn steady(rate: SampleRate) -> rate_change::State {
    rate_change::State::Steady {
        rate,
        adc: device::adc::AdcKind::Unanswered,
    }
}

pub fn with_bench(test: impl FnOnce(&mut Bench<'_>)) {
    let mut ram = UsbRam::new();
    let mut playback = Playback::new();
    let mut capture_ring = FrameRing::new();
    let (buffers, controller) = ram.split();
    let (usb, tick, _counters, gate) = playback.split();
    gate.open();
    let (capture_writer, capture_reader) = capture_ring.split();
    let service = Service::new(
        buffers,
        Descriptors {
            device: &BENCH_DEVICE_DESCRIPTOR,
            strings: &BENCH_STRINGS,
            serial: FACTORY_SERIAL_TEMPLATE,
        },
        Streams {
            playback: PlaybackEndpoint::new(usb, SampleRate::Hz48000),
            capture: CaptureUsb::new(capture_reader, SampleRate::Hz48000),
        },
    );
    let mut bench = Bench {
        service,
        controller,
        tick,
        capture_writer,
        armed: HashMap::new(),
        halted: BTreeSet::new(),
        control_stalled: false,
        address: Address::Default,
        address_after_status: None,
        rate: steady(SampleRate::Hz48000),
        console: Console::default(),
        notices: Vec::new(),
        microframe: 0,
        host_phase: 0,
        next_playback: BufferSlot::First,
        next_capture: BufferSlot::First,
        capture_counter: 0,
        playback_counter: 0,
        log: StreamLog::default(),
        completions_ride_on_the_next_start_of_frame: false,
        completed_since_the_last_start_of_frame: StreamCompletions::NONE,
    };
    test(&mut bench);
}

fn other(slot: BufferSlot) -> BufferSlot {
    match slot {
        BufferSlot::First => BufferSlot::Second,
        BufferSlot::Second => BufferSlot::First,
    }
}

fn halt_index(endpoint: HaltableEndpoint) -> u8 {
    match endpoint {
        HaltableEndpoint::SerialNotification => 0x83,
        HaltableEndpoint::SerialReceive => 0x04,
        HaltableEndpoint::SerialTransmit => 0x84,
    }
}

pub fn numbered_sample(n: i32) -> Frame<StreamSample> {
    [
        StreamSample::from_bits(n << 8),
        StreamSample::from_bits(-(n << 8)),
    ]
}

impl Bench<'_> {
    pub fn deliver(&mut self, event: Event) {
        let reaction = *self
            .service
            .handle(&event, &self.rate.report(), &mut self.console);
        self.notices.extend_from_slice(reaction.notices.as_slice());
        for command in reaction.commands.as_slice() {
            self.execute(*command);
        }
        if matches!(event, Event::BusReset(_) | Event::Detached) {
            self.adopt_the_programmed_address();
        }
    }

    fn execute(&mut self, command: Command) {
        match command {
            Command::ArmOut { entry, room } => {
                let entry = ListEntry::Out(entry);
                assert!(
                    self.armed.insert(entry, room).is_none(),
                    "{entry:?} armed twice without a completion"
                );
                assert_eq!(room.value() % 8, 0, "errata USB.4 room for {entry:?}");
            }
            Command::ArmIn { entry, length } => {
                let entry = ListEntry::In(entry);
                assert!(length <= entry.region().capacity);
                assert!(
                    self.armed.insert(entry, length).is_none(),
                    "{entry:?} armed twice without a completion"
                );
            }
            Command::DisarmOut(entry) => {
                assert!(
                    self.armed.remove(&ListEntry::Out(entry)).is_some(),
                    "{entry:?} was idle"
                );
            }
            Command::DisarmIn(entry) => {
                assert!(
                    self.armed.remove(&ListEntry::In(entry)).is_some(),
                    "{entry:?} was idle"
                );
            }
            Command::StallControl => self.control_stalled = true,
            Command::ReleaseControlStall => self.control_stalled = false,
            Command::Halt(endpoint) => {
                self.halted.insert(halt_index(endpoint));
            }
            Command::ReleaseHalt(endpoint) => {
                self.halted.remove(&halt_index(endpoint));
            }
            Command::ApplyAddress(address) => self.address_after_status = Some(address),
            Command::ResetEndpoints => {
                self.armed.clear();
                self.halted.clear();
                self.control_stalled = false;
            }
        }
    }

    fn adopt_the_programmed_address(&mut self) {
        if let Some(address) = self.address_after_status.take() {
            self.address = address;
        }
    }

    pub fn is_armed(&self, entry: ListEntry) -> bool {
        self.armed.contains_key(&entry)
    }

    pub fn armed_entries(&self) -> BTreeSet<String> {
        self.armed.keys().map(|e| format!("{e:?}")).collect()
    }

    fn take_in_packet(&mut self, entry: InEntry) -> Option<Vec<u8>> {
        let entry = ListEntry::In(entry);
        let length = self.armed.remove(&entry)?;
        Some(self.controller.read(entry, length).collect())
    }

    fn put_out_packet(&mut self, entry: OutEntry, bytes: &[u8]) -> bool {
        let list = ListEntry::Out(entry);
        let Some(room) = self.armed.remove(&list) else {
            return false;
        };
        assert!(bytes.len() <= room.value() as usize, "{entry:?} babble");
        self.controller.write(list, bytes);
        self.deliver(Event::OutCompleted {
            entry,
            untransferred: ByteCount::new(room.value() - bytes.len() as u16),
        });
        true
    }

    pub fn attach_and_reset(&mut self, speed: BusSpeed) {
        self.deliver(Event::Attached);
        self.deliver(Event::BusReset(speed));
    }

    pub fn control_read(&mut self, setup: [u8; 8]) -> Result<Vec<u8>, Stalled> {
        let limit = u16::from_le_bytes([setup[6], setup[7]]) as usize;
        self.deliver(Event::Setup(setup));
        let mut reply = Vec::new();
        loop {
            if self.control_stalled {
                return Err(Stalled);
            }
            let packet = self
                .take_in_packet(InEntry::Control)
                .expect("the device NAKs its IN data stage");
            assert!(packet.len() <= CONTROL_PACKET);
            reply.extend_from_slice(&packet);
            assert!(reply.len() <= limit, "{setup:02X?} replied past wLength");
            self.deliver(Event::InCompleted(InEntry::Control));
            if packet.len() < CONTROL_PACKET || reply.len() == limit {
                break;
            }
        }
        if self.control_stalled {
            return Err(Stalled);
        }
        assert!(
            self.put_out_packet(OutEntry::Control, &[]),
            "the device NAKs the status stage"
        );
        assert!(!self.control_stalled, "status stage stalled");
        Ok(reply)
    }

    pub fn control_write(&mut self, setup: [u8; 8], data: &[u8]) -> Result<(), Stalled> {
        assert_eq!(
            u16::from_le_bytes([setup[6], setup[7]]) as usize,
            data.len()
        );
        self.deliver(Event::Setup(setup));
        for chunk in data.chunks(CONTROL_PACKET) {
            if self.control_stalled {
                return Err(Stalled);
            }
            assert!(
                self.put_out_packet(OutEntry::Control, chunk),
                "the device NAKs its OUT data stage"
            );
        }
        if self.control_stalled {
            return Err(Stalled);
        }
        let address_before_status = self.address;
        let status = self
            .take_in_packet(InEntry::Control)
            .expect("the device NAKs the status stage");
        assert!(status.is_empty(), "a status packet carries no data");
        assert_eq!(
            self.address, address_before_status,
            "the address moved before the status stage completed"
        );
        self.adopt_the_programmed_address();
        self.deliver(Event::InCompleted(InEntry::Control));
        Ok(())
    }

    pub fn reported_rate(&self) -> SampleRate {
        match self.rate {
            rate_change::State::Steady { rate, .. } => rate,
            other => panic!("the bench keeps the rate machine steady, not {other:?}"),
        }
    }

    pub fn change_rate_as_the_rate_machine_would(&mut self, rate: SampleRate) {
        self.rate = steady(rate);
        self.service.reset_streams_for_rate(rate);
    }

    pub fn microframe(&mut self, playing: bool, capturing: bool) {
        let rate = self.reported_rate();
        let frames_per_microframe = rate.hz() / 8000;
        let mut block = [numbered_sample(0); 32];
        for frame in block.iter_mut().take(frames_per_microframe as usize) {
            *frame = numbered_sample(self.capture_counter);
            self.capture_counter += 1;
        }
        self.capture_writer
            .push(&block[..frames_per_microframe as usize]);
        let mut out = [[StreamSample::from_bits(0); 2]; MAX_FRAMES];
        self.tick.render(rate.frames_per_tick(), &mut out);
        let tick_frames = match rate.frames_per_tick() {
            dsp::FramesPerTick::Six => 6u32,
            dsp::FramesPerTick::Twelve => 12,
            dsp::FramesPerTick::TwentyFour => 24,
        };
        self.log.rendered_nonsilent_frames += out[..tick_frames as usize]
            .iter()
            .filter(|frame| frame[0].to_bits() != 0)
            .count() as u64;

        let completed = core::mem::replace(
            &mut self.completed_since_the_last_start_of_frame,
            StreamCompletions::NONE,
        );
        self.deliver(Event::StartOfFrame(FrameStart {
            positions: DmaPositions {
                playback: TxDmaPosition {
                    frames_queued: Frames::new(tick_frames),
                },
                capture: Frames::new(0),
            },
            completed,
        }));

        if playing {
            let feedback = self
                .log
                .feedback
                .last()
                .copied()
                .unwrap_or(FeedbackValue::nominal(rate).samples_per_microframe_16_16());
            self.host_phase += u64::from(feedback);
            let frames = (self.host_phase >> 16) as usize;
            self.host_phase &= 0xFFFF;
            let mut bytes = Vec::with_capacity(frames * 8);
            for _ in 0..frames {
                self.playback_counter += 1;
                for sample in numbered_sample(self.playback_counter) {
                    bytes.extend_from_slice(&sample.to_bits().to_le_bytes());
                }
            }
            let slot = self.next_playback;
            if self.put_playback_packet(slot, &bytes) {
                self.next_playback = other(slot);
                self.log.playback_frames_sent += frames as u64;
            } else {
                self.log.playback_packets_lost += 1;
            }
            if self.microframe % FEEDBACK_POLL_MICROFRAMES == 0 {
                if let Some(packet) = self.take_in_packet(InEntry::Feedback) {
                    self.log
                        .feedback
                        .push(u32::from_le_bytes(packet.try_into().unwrap()));
                    self.complete_in(InEntry::Feedback);
                }
            }
        }
        if capturing {
            let slot = self.next_capture;
            match self.take_in_packet(InEntry::Capture(slot)) {
                Some(packet) => {
                    self.log.capture_packets.push(packet);
                    self.next_capture = other(slot);
                    self.complete_in(InEntry::Capture(slot));
                }
                None => self.log.capture_polls_unanswered += 1,
            }
        }
        self.microframe += 1;
    }

    fn put_playback_packet(&mut self, slot: BufferSlot, bytes: &[u8]) -> bool {
        if !self.completions_ride_on_the_next_start_of_frame {
            return self.put_out_packet(OutEntry::Playback(slot), bytes);
        }
        let list = ListEntry::Out(OutEntry::Playback(slot));
        let Some(room) = self.armed.remove(&list) else {
            return false;
        };
        assert!(bytes.len() <= room.value() as usize, "playback babble");
        self.controller.write(list, bytes);
        let arrival = PlaybackArrival {
            slot,
            untransferred: ByteCount::new(room.value() - bytes.len() as u16),
        };
        let free = self
            .completed_since_the_last_start_of_frame
            .playback
            .iter_mut()
            .find(|held| held.is_none())
            .expect("at most two playback packets land between two starts of frame");
        *free = Some(arrival);
        true
    }

    fn complete_in(&mut self, entry: InEntry) {
        if !self.completions_ride_on_the_next_start_of_frame {
            self.deliver(Event::InCompleted(entry));
            return;
        }
        let completed = &mut self.completed_since_the_last_start_of_frame;
        match entry {
            InEntry::Feedback => completed.feedback_sent = true,
            InEntry::Capture(slot) => {
                let free = completed
                    .capture_sent
                    .iter_mut()
                    .find(|held| held.is_none())
                    .expect("at most two capture packets leave between two starts of frame");
                *free = Some(slot);
            }
            InEntry::Control | InEntry::Serial => self.deliver(Event::InCompleted(entry)),
        }
    }

    pub fn send_console_bytes(&mut self, bytes: &[u8]) -> bool {
        self.put_out_packet(OutEntry::Serial, bytes)
    }

    pub fn read_console_packet(&mut self) -> Option<Vec<u8>> {
        let packet = self.take_in_packet(InEntry::Serial)?;
        self.deliver(Event::InCompleted(InEntry::Serial));
        Some(packet)
    }
}

impl Bench<'_> {
    pub fn host_completes(&mut self, entry: ListEntry, fill: u8) {
        let room = self.armed.remove(&entry);
        match entry {
            ListEntry::Out(out) => {
                let room = room.unwrap_or(ByteCount::ZERO);
                let landed = room.value() / 2;
                self.controller.write(entry, &vec![fill; landed as usize]);
                self.deliver(Event::OutCompleted {
                    entry: out,
                    untransferred: ByteCount::new(room.value() - landed),
                });
            }
            ListEntry::In(sent) => self.deliver(Event::InCompleted(sent)),
            ListEntry::Setup => {}
        }
    }
}
