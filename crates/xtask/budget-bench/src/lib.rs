#![no_std]

use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicU32, Ordering};

use device::adc::AdcKind;
use device::rate_change;
use device::usb::answer::Descriptors;
use device::console::{ConsoleRings, ConsoleThread, ConsoleUsb, OUTBOUND_PACKET_BYTES};
use device::usb::dispatch::Streams;
use device::usb::endpoint::{
    BufferSlot, ByteCount, ControllerView, DmaPositions, InEntry, ListEntry, OutEntry, UsbRam,
};
use device::usb::service::Service;
use device::usb::{BENCH_DEVICE_DESCRIPTOR, BENCH_STRINGS, FACTORY_SERIAL_TEMPLATE};
use dsp::{
    Band, BandFrequency, BandGain, BandQ, CaptureBlock, CaptureGainTargets, CoefficientExchange,
    CoefficientTick, Crossfade, DspMode, DspModeSwitch, FilterKind, FramesPerTick, Gain, I2sWord,
    InputMute, InputMutes, MeterTick, Meters, MonitorMix, MonitorTargets, PeqPreset, Pipeline,
    PlaybackBlock, PresetBand, PublishCount, PublishesSeen, RateFollower, RateSwitch, Sample,
    SampleRate, StreamSample, TickControls,
};
use stream::{
    CaptureUsb, DMA_BUFFER_FRAMES, DmaTransport, FrameRing, Playback, PlaybackEndpoint,
    PlaybackTick, RingWriter, TickBlocks, XferCount,
};

#[allow(dead_code, unused_imports, unused_macros)]
mod firmware {
    pub mod handovers {
        pub static USB_NOTICES: device::usb::handover::UsbNotices =
            device::usb::handover::UsbNotices::new();
    }
    #[path = "../../../../board/src/firmware/map.rs"]
    pub mod map;
    #[path = "../../../../board/src/firmware/mmio.rs"]
    pub mod mmio;
    #[path = "../../../../board/src/firmware/usb_controller.rs"]
    pub mod usb_controller;
}

use firmware::map::{
    USB1_RAM_ENDPOINT_LIST, USB1_RAM_LIST_ACTIVE, USB1_RAM_LIST_NBYTES_MASK,
    USB1_RAM_LIST_NBYTES_SHIFT, USBHSD_DEVCMDSTAT, USBHSD_DEVCMDSTAT_DRES_C,
    USBHSD_DEVCMDSTAT_SETUP, USBHSD_DEVCMDSTAT_SPEED_HIGH, USBHSD_DEVCMDSTAT_SPEED_SHIFT,
    USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR, USBHSD_EPINUSE, USBHSD_INTSTAT, USBHSD_INTSTAT_DEV_INT,
    USBHSD_INTSTAT_EP0IN, USBHSD_INTSTAT_EP0OUT, USBHSD_INTSTAT_FRAME_INT,
};
use firmware::usb_controller::{attach, serve, usb_device_commands, usb_device_interrupt};

struct TickBench {
    pipeline: Pipeline,
    crossfade: Crossfade,
    frames: FramesPerTick,
    publishes: PublishesSeen,
    capture_gains: [Gain; 2],
    monitor: MonitorMix,
    inputs: [InputMute; 2],
    rate: RateFollower,
    playback: PlaybackTick<'static>,
    capture: RingWriter<'static>,
    coefficients: CoefficientTick<'static>,
    meters: MeterTick<'static>,
    playback_block: PlaybackBlock,
    capture_block: CaptureBlock,
}

struct ConsoleBench {
    usb: ConsoleUsb<'static>,
    thread: ConsoleThread<'static>,
}

impl ConsoleBench {
    fn thread_queues_a_full_packet(&mut self) {
        for _ in 0..OUTBOUND_PACKET_BYTES {
            require(self.thread.to_host.push(b'x'));
        }
        self.thread.to_host.flush();
    }

    fn thread_drains_what_the_host_sent(&mut self) -> usize {
        let mut drained = 0;
        while self.thread.from_host.next_byte().is_some() {
            drained += 1;
        }
        drained
    }
}

#[repr(C)]
struct UsbBench {
    service: Service<'static>,
    rate: rate_change::State,
    controller: ControllerView<'static>,
    control_data_to_send: &'static [u8],
    console: ConsoleBench,
    positions: DmaPositions,
    nominal_frames: usize,
    host_microframes: usize,
    largest_playback_packet: usize,
    largest_capture_packet: usize,
    feedback_reads: usize,
}

struct Bench {
    tick: TickBench,
    usb: UsbBench,
}

const CONSOLE_PACKET_BYTES: usize = 512;
const STEREO_FRAME_BYTES: usize = 8;
const LARGEST_HOST_PACKET_FRAMES: usize = 25;
const PLAYBACK_INTERFACE: u16 = 1;
const CAPTURE_INTERFACE: u16 = 2;
const ASSIGNED_ADDRESS: u16 = 5;
const STREAMING_SETTING: u16 = 1;
// WHY: USB RAM is 16 KB at 0x4010_0000, the only memory USB1 can write, UM11126 section 44.1; the bench claims it where the image does so the driver code is the same
const USB_RAM_ADDRESS: usize = 0x4010_0000;
const LOG2_LIST_BYTES_PER_PHYSICAL_ENDPOINT: u16 = 3;
const DEVICE_ADDRESS_MASK: u32 = 0x7F;
const CONTROL_OUT: ListEntry = ListEntry::Out(OutEntry::Control);
const CONTROL_IN: ListEntry = ListEntry::In(InEntry::Control);
const PLAYBACK_FIRST: ListEntry = ListEntry::Out(OutEntry::Playback(BufferSlot::First));
const PLAYBACK_SECOND: ListEntry = ListEntry::Out(OutEntry::Playback(BufferSlot::Second));
const CAPTURE_FIRST: ListEntry = ListEntry::In(InEntry::Capture(BufferSlot::First));
const FEEDBACK: ListEntry = ListEntry::In(InEntry::Feedback);
const SERIAL_OUT: ListEntry = ListEntry::Out(OutEntry::Serial);
const SERIAL_IN: ListEntry = ListEntry::In(InEntry::Serial);

const MONITORED_AT_MINUS_THREE_DB: MonitorMix = MonitorMix {
    playback: Gain::from_q5_27(0x05A8_2799),
    inputs: [Gain::from_q5_27(0x05A8_2799); 2],
};

const BAND_PLAN: [(FilterKind, f64, f64); 5] = [
    (FilterKind::LowShelf, 80.0, 4.0),
    (FilterKind::Peaking, 250.0, -3.0),
    (FilterKind::Peaking, 1_000.0, 6.0),
    (FilterKind::Peaking, 4_000.0, -6.0),
    (FilterKind::HighShelf, 10_000.0, 3.0),
];

static DSP_MODE: DspModeSwitch = DspModeSwitch::new();
static CAPTURE_GAINS: CaptureGainTargets = CaptureGainTargets::new();
static MONITOR: MonitorTargets = MonitorTargets::new();
static INPUT_MUTES: InputMutes = InputMutes::new();
static RATE: RateSwitch = RateSwitch::at(SampleRate::Hz48000);
static AUDIO: DmaTransport = DmaTransport::silent();
static RECEIVE_XFERCFG: AtomicU32 = AtomicU32::new(0);
static FRAMES_RECEIVED: AtomicU32 = AtomicU32::new(0);
static PUBLISHES: PublishCount = PublishCount::new();

const FACTORY_LAST_TRANSFER: u32 = 191;
const XFERCOUNT_SHIFT: u32 = 16;
static mut PLAYBACK: Playback = Playback::new();
static mut CAPTURE: FrameRing = FrameRing::new();
static mut METERS: Meters = Meters::new();
static mut COEFFICIENTS: CoefficientExchange = CoefficientExchange::flat_at(SampleRate::Hz48000);
static mut BENCH: Option<Bench> = None;
static mut CONSOLE: ConsoleRings = ConsoleRings::new();

fn five_band_preset() -> PeqPreset {
    let mut preset = PeqPreset::FLAT;
    for (slot, (kind, hz, db)) in preset.bands.iter_mut().zip(BAND_PLAN) {
        if let (Ok(frequency), Ok(gain), Ok(q)) = (
            BandFrequency::from_hz(hz),
            BandGain::from_db(db),
            BandQ::new(1.0),
        ) {
            *slot = PresetBand {
                band: Band {
                    kind,
                    frequency,
                    gain,
                    q,
                },
                enabled: true,
            };
        }
    }
    preset
}

fn noise_word(state: &mut u32) -> i32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 2) as i32 - (1 << 29)
}

fn rate_for(frames_code: u32) -> SampleRate {
    match frames_code {
        6 => SampleRate::Hz48000,
        12 => SampleRate::Hz96000,
        _ => SampleRate::Hz192000,
    }
}

fn bench() -> &'static mut Bench {
    match unsafe { &mut *addr_of_mut!(BENCH) } {
        Some(bench) => bench,
        None => refuse(),
    }
}

fn refuse() -> ! {
    unsafe { core::arch::asm!("udf #0", options(noreturn)) }
}

fn require(holds: bool) {
    if !holds {
        refuse();
    }
}

const fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
    let [value_low, value_high] = value.to_le_bytes();
    let [index_low, index_high] = index.to_le_bytes();
    let [length_low, length_high] = length.to_le_bytes();
    [
        request_type,
        request,
        value_low,
        value_high,
        index_low,
        index_high,
        length_low,
        length_high,
    ]
}

const SET_ADDRESS: [u8; 8] = setup(0x00, 0x05, ASSIGNED_ADDRESS, 0, 0);
const SET_CONFIGURATION_1: [u8; 8] = setup(0x00, 0x09, 1, 0, 0);
const OPEN_PLAYBACK: [u8; 8] = setup(0x01, 0x0B, STREAMING_SETTING, PLAYBACK_INTERFACE, 0);
const OPEN_CAPTURE: [u8; 8] = setup(0x01, 0x0B, STREAMING_SETTING, CAPTURE_INTERFACE, 0);
const GET_CONFIGURATION_DESCRIPTOR: [u8; 8] = setup(0x80, 0x06, 0x0200, 0, 0xFF);
const SET_CUR_SAMPLING_FREQUENCY: [u8; 8] = setup(0x21, 0x01, 0x0100, 0x0A00, 4);
const SAMPLING_FREQUENCY_192_KHZ: [u8; 4] = 192_000u32.to_le_bytes();

fn list_word(entry: ListEntry) -> firmware::mmio::Register {
    USB1_RAM_ENDPOINT_LIST.bytes_after(entry.list_offset() as usize)
}

fn endpoint_bit(entry: ListEntry) -> u32 {
    1 << (entry.list_offset() >> LOG2_LIST_BYTES_PER_PHYSICAL_ENDPOINT)
}

fn armed(entry: ListEntry) -> Option<ByteCount> {
    let word = list_word(entry).read();
    (word & USB1_RAM_LIST_ACTIVE != 0).then(|| {
        ByteCount::new(((word >> USB1_RAM_LIST_NBYTES_SHIFT) & USB1_RAM_LIST_NBYTES_MASK) as u16)
    })
}

fn armed_or_refuse(entry: ListEntry) -> ByteCount {
    match armed(entry) {
        Some(bytes) => bytes,
        None => refuse(),
    }
}

fn host_finishes(entry: ListEntry, left: ByteCount) {
    let word = list_word(entry).read()
        & !(USB1_RAM_LIST_ACTIVE | (USB1_RAM_LIST_NBYTES_MASK << USB1_RAM_LIST_NBYTES_SHIFT));
    list_word(entry).write(word | (u32::from(left.value()) << USB1_RAM_LIST_NBYTES_SHIFT));
}

fn slot_in_use(first: ListEntry) -> BufferSlot {
    match USBHSD_EPINUSE.read() & endpoint_bit(first) {
        0 => BufferSlot::First,
        _ => BufferSlot::Second,
    }
}

fn controller_moves_to_the_other_buffer(first: ListEntry) {
    USBHSD_EPINUSE.write(USBHSD_EPINUSE.read() ^ endpoint_bit(first));
}

fn controller_raises(interrupts: u32) {
    let status = USBHSD_DEVCMDSTAT.read();
    USBHSD_DEVCMDSTAT.write(status & !USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR);
    USBHSD_INTSTAT.write(USBHSD_INTSTAT.read() | interrupts);
}

fn controller_clears_what_the_driver_acknowledged(raised: u32) {
    let stream_bits =
        endpoint_bit(PLAYBACK_FIRST) | endpoint_bit(FEEDBACK) | endpoint_bit(CAPTURE_FIRST);
    USBHSD_INTSTAT.write(match raised & USBHSD_INTSTAT_FRAME_INT {
        0 => raised & stream_bits,
        _ => 0,
    });
}

impl UsbBench {
    fn interrupt(&mut self) {
        let raised = USBHSD_INTSTAT.read();
        usb_device_interrupt(
            &mut self.service,
            &self.rate.report(),
            &self.positions,
            &mut self.console.usb,
        );
        controller_clears_what_the_driver_acknowledged(raised);
    }

    fn host_resets_the_bus_at_high_speed(&mut self) {
        controller_raises(USBHSD_INTSTAT_DEV_INT);
        USBHSD_DEVCMDSTAT.write(
            USBHSD_DEVCMDSTAT.read()
                | USBHSD_DEVCMDSTAT_DRES_C
                | (USBHSD_DEVCMDSTAT_SPEED_HIGH << USBHSD_DEVCMDSTAT_SPEED_SHIFT),
        );
        self.interrupt();
    }

    fn host_sends_setup(&mut self, packet: [u8; 8]) {
        controller_raises(USBHSD_INTSTAT_EP0OUT);
        self.controller.write(ListEntry::Setup, &packet);
        USBHSD_DEVCMDSTAT.write(USBHSD_DEVCMDSTAT.read() | USBHSD_DEVCMDSTAT_SETUP);
    }

    fn control_write_without_data(&mut self, packet: [u8; 8]) {
        self.host_sends_setup(packet);
        self.interrupt();
        require(armed(CONTROL_IN) == Some(ByteCount::ZERO));
        host_finishes(CONTROL_IN, ByteCount::ZERO);
        controller_raises(USBHSD_INTSTAT_EP0IN);
        self.interrupt();
    }

    fn enumerate_and_open_both_streams(&mut self) {
        self.host_resets_the_bus_at_high_speed();
        self.control_write_without_data(SET_ADDRESS);
        require(USBHSD_DEVCMDSTAT.read() & DEVICE_ADDRESS_MASK == u32::from(ASSIGNED_ADDRESS));
        self.control_write_without_data(SET_CONFIGURATION_1);
        self.control_write_without_data(OPEN_PLAYBACK);
        self.control_write_without_data(OPEN_CAPTURE);
        require(armed(PLAYBACK_FIRST).is_some());
        require(armed(PLAYBACK_SECOND).is_some());
        require(armed(SERIAL_OUT).is_some());
    }

    fn host_packet_frames(&self) -> usize {
        match self.host_microframes % 2 {
            0 => self.nominal_frames + 1,
            _ => self.nominal_frames - 1,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_init(frames_code: u32, monitored: u32, five_bands: u32) {
    let rate = rate_for(frames_code);
    let frames = rate.frames_per_tick();
    let (playback, capture, meters, coefficients, ram) = unsafe {
        let playback = &mut *addr_of_mut!(PLAYBACK);
        let capture = &mut *addr_of_mut!(CAPTURE);
        let meters = &mut *addr_of_mut!(METERS);
        let coefficients = &mut *addr_of_mut!(COEFFICIENTS);
        let ram = &mut *(USB_RAM_ADDRESS as *mut UsbRam);
        *playback = Playback::new();
        *capture = FrameRing::new();
        *meters = Meters::new();
        *coefficients = CoefficientExchange::flat_at(rate);
        *ram = UsbRam::new();
        (playback, capture, meters, coefficients, ram)
    };
    DSP_MODE.request(DspMode::Bypass);
    let (playback_usb, playback_tick, _, playback_gate) = playback.split();
    playback_gate.open();
    let (capture_writer, capture_reader) = capture.split();
    let (meter_tick, _) = meters.split();
    let (mut publisher, coefficient_tick) = coefficients.split();
    let (buffers, controller) = ram.split();
    if five_bands != 0 {
        publisher.change(&five_band_preset(), rate);
    }
    let mut state = 0x1234_5678;
    let mut capture_block = CaptureBlock::new();
    for frame in capture_block.input.iter_mut() {
        *frame = [0, 0].map(|_: i32| I2sWord::from_bits(noise_word(&mut state)));
    }
    the_dma_reads_the_transmit_buffers(&AUDIO);
    FRAMES_RECEIVED.store(frames.count() as u32, Ordering::Relaxed);
    RECEIVE_XFERCFG.store(xfercfg_at(frames.count() as u32), Ordering::Relaxed);
    MONITOR.publish(if monitored != 0 {
        MONITORED_AT_MINUS_THREE_DB
    } else {
        MonitorMix::OFF
    });
    PUBLISHES.published();
    let service = Service::new(
        buffers,
        Descriptors {
            device: &BENCH_DEVICE_DESCRIPTOR,
            strings: &BENCH_STRINGS,
            serial: FACTORY_SERIAL_TEMPLATE,
        },
        Streams {
            playback: PlaybackEndpoint::new(playback_usb, rate),
            capture: CaptureUsb::new(capture_reader, rate),
        },
    );
    attach(|| ());
    let usb = UsbBench {
        service,
        controller,
        rate: rate_change::State::Steady {
            rate,
            adc: AdcKind::Unanswered,
        },
        control_data_to_send: &[],
        console: {
            let (usb, thread) = unsafe { &mut *addr_of_mut!(CONSOLE) }.split();
            ConsoleBench { usb, thread }
        },
        positions: start_of_frame_halfway_through_the_block(frames),
        nominal_frames: frames.count(),
        host_microframes: 0,
        largest_playback_packet: 0,
        largest_capture_packet: 0,
        feedback_reads: 0,
    };
    let tick = TickBench {
        pipeline: Pipeline::default(),
        crossfade: Crossfade::new(),
        frames,
        publishes: PublishesSeen::of(&PUBLISHES),
        capture_gains: CAPTURE_GAINS.latest(),
        monitor: MONITOR.latest_or(MonitorMix::OFF),
        inputs: INPUT_MUTES.latest(),
        rate: RateFollower::of(&RATE),
        playback: playback_tick,
        capture: capture_writer,
        coefficients: coefficient_tick,
        meters: meter_tick,
        playback_block: PlaybackBlock::new(),
        capture_block,
    };
    unsafe { *addr_of_mut!(BENCH) = Some(Bench { tick, usb }) };
    bench().usb.enumerate_and_open_both_streams();
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_request(on: u32) {
    DSP_MODE.request(if on != 0 {
        DspMode::On
    } else {
        DspMode::Bypass
    });
}

trait IntoPositions {
    fn into_positions(self) -> DmaPositions;
}

impl IntoPositions for stream::DmaFill {
    fn into_positions(self) -> DmaPositions {
        DmaPositions {
            playback: self.playback,
            capture: self.capture,
        }
    }
}

fn the_dma_reads_the_transmit_buffers(transport: &'static DmaTransport) {
    core::hint::black_box(transport);
}

fn xfercfg_at(frames_received: u32) -> u32 {
    let frame = frames_received % DMA_BUFFER_FRAMES as u32;
    (FACTORY_LAST_TRANSFER - 2 * frame) << XFERCOUNT_SHIFT
}

fn receive_reading() -> XferCount {
    XferCount::from_xfercfg(core::hint::black_box(
        RECEIVE_XFERCFG.load(Ordering::Relaxed),
    ))
}

fn start_of_frame_halfway_through_the_block(frames: FramesPerTick) -> DmaPositions {
    let received = FRAMES_RECEIVED.load(Ordering::Relaxed) - frames.count() as u32 / 2;
    let position = XferCount::from_xfercfg(xfercfg_at(received)).frame_in_progress();
    AUDIO.cursor.fill_at(position).into_positions()
}

fn the_dma_receives_one_tick_of_frames(frames: FramesPerTick) {
    let received = FRAMES_RECEIVED.load(Ordering::Relaxed) + frames.count() as u32;
    FRAMES_RECEIVED.store(received, Ordering::Relaxed);
    let frame = received % DMA_BUFFER_FRAMES as u32;
    RECEIVE_XFERCFG.store(xfercfg_at(received), Ordering::Relaxed);
    for word in 0..2 * frames.count() {
        AUDIO.received.store_word(
            (2 * frame as usize + word) % (2 * DMA_BUFFER_FRAMES),
            noise_word(&mut (received ^ word as u32 | 1)) as u32,
        );
    }
}

#[cold]
#[inline(never)]
fn adopt_what_thread_mode_published(tick: &mut TickBench) {
    tick.capture_gains = CAPTURE_GAINS.latest();
    tick.monitor = MONITOR.latest_or(tick.monitor);
    tick.inputs = INPUT_MUTES.latest();
    if let Some(rate) = tick.rate.changed(&RATE) {
        tick.frames = rate.frames_per_tick();
        tick.pipeline = Pipeline::default();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_tick() {
    let tick = &mut bench().tick;
    let reading = receive_reading();
    if tick.publishes.any_new(&PUBLISHES) {
        adopt_what_thread_mode_published(tick);
    }
    let frames = tick.frames;
    let blocks = TickBlocks::receive(reading, frames, &AUDIO, &mut tick.capture_block.input);
    let mix = tick.crossfade.tick(&DSP_MODE);
    tick.playback.render(frames, &mut tick.playback_block.input);
    let controls = TickControls {
        frames,
        mix,
        coefficients: tick.coefficients.adopt(),
        monitor: tick.monitor,
        inputs: tick.inputs,
        capture_gains: tick.capture_gains,
    };
    tick.pipeline.tick(
        &controls,
        &mut tick.playback_block,
        &mut tick.capture_block,
        &mut tick.meters,
    );
    blocks.transmit(&tick.playback_block.output, &AUDIO);
    let captured = tick.capture_block.output.split_at(frames.count()).0;
    tick.capture.push(captured);
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_event() {
    bench().usb.interrupt();
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_console_event() {
    bench().usb.interrupt();
}

fn converters_run_ahead_of_the_host_clock(bench: &mut Bench) {
    if bench.usb.host_microframes.is_multiple_of(2) {
        bench.tick.capture.push(&[[StreamSample::SILENCE; 2]]);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_starts_a_microframe() {
    let bench = bench();
    converters_run_ahead_of_the_host_clock(bench);
    the_dma_receives_one_tick_of_frames(bench.tick.frames);
    bench.usb.positions = start_of_frame_halfway_through_the_block(bench.tick.frames);
    controller_raises(USBHSD_INTSTAT_FRAME_INT);
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_starts_a_microframe_with_console_bytes_waiting() {
    let bench = bench();
    converters_run_ahead_of_the_host_clock(bench);
    bench.usb.console.thread_queues_a_full_packet();
    controller_raises(USBHSD_INTSTAT_FRAME_INT);
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_sends_a_playback_packet() {
    let usb = &mut bench().usb;
    let frames = usb.host_packet_frames();
    usb.host_microframes += 1;
    let entry = ListEntry::Out(OutEntry::Playback(slot_in_use(PLAYBACK_FIRST)));
    let room = armed_or_refuse(entry);
    require(room.value().is_multiple_of(8));
    let mut bytes = [0u8; LARGEST_HOST_PACKET_FRAMES * STEREO_FRAME_BYTES];
    let mut state = 0x9E37_79B9 ^ usb.host_microframes as u32;
    for word in bytes.as_chunks_mut::<4>().0 {
        word.copy_from_slice(&(noise_word(&mut state) & !0xFF).to_le_bytes());
    }
    let length = frames * STEREO_FRAME_BYTES;
    require(length <= room.value() as usize);
    usb.controller.write(entry, &bytes[..length]);
    usb.largest_playback_packet = usb.largest_playback_packet.max(frames);
    host_finishes(entry, ByteCount::new(room.value() - length as u16));
    controller_moves_to_the_other_buffer(PLAYBACK_FIRST);
    controller_raises(endpoint_bit(PLAYBACK_FIRST));
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_reads_feedback() {
    let usb = &mut bench().usb;
    let length = armed_or_refuse(FEEDBACK);
    require(usb.controller.read(FEEDBACK, length).count() == 4);
    usb.feedback_reads += 1;
    host_finishes(FEEDBACK, ByteCount::ZERO);
    controller_raises(endpoint_bit(FEEDBACK));
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_reads_a_capture_packet() {
    let usb = &mut bench().usb;
    let entry = ListEntry::In(InEntry::Capture(slot_in_use(CAPTURE_FIRST)));
    let length = armed_or_refuse(entry);
    let frames = length.value() as usize / STEREO_FRAME_BYTES;
    require(frames + 1 >= usb.nominal_frames);
    usb.largest_capture_packet = usb.largest_capture_packet.max(frames);
    host_finishes(entry, ByteCount::ZERO);
    controller_moves_to_the_other_buffer(CAPTURE_FIRST);
    controller_raises(endpoint_bit(CAPTURE_FIRST));
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_reads_the_console_packet() {
    let usb = &mut bench().usb;
    let length = armed_or_refuse(SERIAL_IN);
    require(length.value() as usize == OUTBOUND_PACKET_BYTES);
    host_finishes(SERIAL_IN, ByteCount::ZERO);
    controller_raises(endpoint_bit(SERIAL_IN));
    usb.interrupt();
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_sends_console_bytes() {
    let usb = &mut bench().usb;
    let drained = usb.console.thread_drains_what_the_host_sent();
    require(drained == 0 || drained == CONSOLE_PACKET_BYTES);
    if drained != 0 {
        usb.interrupt();
    }
    let room = armed_or_refuse(SERIAL_OUT);
    require(room.value() as usize == CONSOLE_PACKET_BYTES);
    usb.controller
        .write(SERIAL_OUT, &[b'y'; CONSOLE_PACKET_BYTES]);
    host_finishes(SERIAL_OUT, ByteCount::ZERO);
    controller_raises(endpoint_bit(SERIAL_OUT));
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_reads_the_configuration_descriptor() {
    bench().usb.host_sends_setup(GET_CONFIGURATION_DESCRIPTOR);
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_sets_the_sampling_frequency() {
    let usb = &mut bench().usb;
    usb.control_data_to_send = &SAMPLING_FREQUENCY_192_KHZ;
    usb.host_sends_setup(SET_CUR_SAMPLING_FREQUENCY);
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_reselects_capture() {
    bench().usb.host_sends_setup(OPEN_CAPTURE);
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_host_continues_the_control_transfer() {
    let usb = &mut bench().usb;
    if armed(CONTROL_IN).is_some() {
        host_finishes(CONTROL_IN, ByteCount::ZERO);
        controller_raises(USBHSD_INTSTAT_EP0IN);
    } else if let Some(room) = armed(CONTROL_OUT) {
        let data = core::mem::take(&mut usb.control_data_to_send);
        usb.controller.write(CONTROL_OUT, data);
        host_finishes(
            CONTROL_OUT,
            ByteCount::new(room.value() - data.len() as u16),
        );
        controller_raises(USBHSD_INTSTAT_EP0OUT);
    } else {
        controller_raises(USBHSD_INTSTAT_EP0IN);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn budget_usb_check() {
    let usb = &bench().usb;
    require(usb.largest_playback_packet == usb.nominal_frames + 1);
    require(usb.largest_capture_packet == usb.nominal_frames + 1);
    require(usb.feedback_reads > 0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    refuse()
}
