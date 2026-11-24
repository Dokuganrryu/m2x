#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use dsp::SampleRate;

use super::answer::{ControlContext, Descriptors, Effect};
use super::configuration::BusSpeed;
use super::control::{self, CONTROL_PACKET_BYTES};
use super::dispatch::{DataEndpoints, SerialPipe, Streams};
use super::endpoint::{
    Buffers, ByteCount, Command, Commands, Event, InEntry, InSlot, OutEntry, OutSlot,
};
use super::function::{self, AudioAndSerial, Bus, Configuration};
use super::request::{Address, DataTerminal, LineCoding};
use crate::outputs::{Filler, OutputList};
use crate::rate_change;

pub const MOST_NOTICES_PER_EVENT: usize = 4;

const CONTROL_ROOM: ByteCount = ByteCount::new(CONTROL_PACKET_BYTES as u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    RateChange(rate_change::Event),
    DataTerminal(DataTerminal),
    FadeOutputsToSilence,
    ReleaseOutputSilence,
    SuspendPhantom,
    ResumePhantom,
}

impl Filler for Notice {
    const FILLER: Self = Notice::ReleaseOutputSilence;
}

pub type Notices = OutputList<Notice, MOST_NOTICES_PER_EVENT>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reaction {
    pub commands: Commands,
    pub notices: Notices,
}

impl Reaction {
    pub const NONE: Self = Self {
        commands: Commands::NONE,
        notices: Notices::NONE,
    };
}

pub struct Service<'a> {
    control: control::State<'a>,
    function: function::State,
    line_coding: LineCoding,
    descriptors: Descriptors<'a>,
    control_out: OutSlot<'a>,
    control_in: InSlot<'a>,
    data: DataEndpoints<'a>,
    streams: Streams<'a>,
    reaction: Reaction,
}

const fn configured(bus: Bus) -> Option<Configuration> {
    match bus {
        Bus::Configured(configuration) => Some(configuration),
        Bus::Powered | Bus::Default(_) | Bus::Addressed(_) => None,
    }
}

const fn streaming_audio(state: function::State) -> Option<AudioAndSerial> {
    match state {
        function::State::Awake(bus) => match configured(bus) {
            Some(Configuration::HighSpeed(audio)) => Some(audio),
            Some(Configuration::FullSpeed(_)) | None => None,
        },
        function::State::Detached | function::State::Suspended(_) => None,
    }
}

const fn serial_speed(state: function::State) -> Option<BusSpeed> {
    match state {
        function::State::Awake(bus) | function::State::Suspended(bus) => match configured(bus) {
            Some(configuration) => Some(configuration.speed()),
            None => None,
        },
        function::State::Detached => None,
    }
}

impl<'a> Service<'a> {
    pub fn new(buffers: Buffers<'a>, descriptors: Descriptors<'a>, streams: Streams<'a>) -> Self {
        let (data, control_out, control_in) = DataEndpoints::new(buffers);
        Self {
            control: control::State::INITIAL,
            function: function::State::INITIAL,
            line_coding: LineCoding::CONSOLE_DEFAULT,
            descriptors,
            control_out,
            control_in,
            data,
            streams,
            reaction: Reaction::NONE,
        }
    }

    pub const fn function(&self) -> function::State {
        self.function
    }

    pub const fn line_coding(&self) -> LineCoding {
        self.line_coding
    }

    pub const fn reaction(&self) -> &Reaction {
        &self.reaction
    }

    pub fn reset_streams_for_rate(&mut self, rate: SampleRate) {
        self.streams.playback.reset_for_new_rate(rate);
        self.streams.capture.reset_for_new_rate(rate);
    }

    #[inline(never)]
    pub fn handle(
        &mut self,
        event: &Event,
        rate: &rate_change::ClockReport,
        pipe: &mut impl SerialPipe,
    ) -> &Reaction {
        self.reaction.commands.clear();
        self.reaction.notices.clear();
        match *event {
            Event::StartOfFrame(frame) => {
                let audio = streaming_audio(self.function);
                for arrival in frame.completed.playback.iter().flatten() {
                    self.data.playback_completed(
                        arrival.slot,
                        arrival.untransferred,
                        audio,
                        &mut self.streams,
                        &mut self.reaction.commands,
                    );
                }
                if frame.completed.feedback_sent {
                    self.data.in_completed(InEntry::Feedback);
                }
                for slot in frame.completed.capture_sent.iter().flatten() {
                    self.data.in_completed(InEntry::Capture(*slot));
                }
                self.data.start_of_frame(
                    frame.positions,
                    audio,
                    serial_speed(self.function),
                    &mut self.streams,
                    pipe,
                    &mut self.reaction.commands,
                );
            }
            Event::OutCompleted {
                entry: OutEntry::Playback(slot),
                untransferred,
            } => self.data.playback_completed(
                slot,
                untransferred,
                streaming_audio(self.function),
                &mut self.streams,
                &mut self.reaction.commands,
            ),
            Event::OutCompleted {
                entry: OutEntry::Serial,
                untransferred,
            } => self.data.serial_received(
                untransferred,
                serial_speed(self.function),
                pipe,
                &mut self.reaction.commands,
            ),
            Event::InCompleted(
                entry @ (InEntry::Feedback | InEntry::Capture(_) | InEntry::Serial),
            ) => self.data.in_completed(entry),
            Event::Setup(packet) => {
                self.control_out.retire(&mut self.reaction.commands);
                self.control_in.retire(&mut self.reaction.commands);
                self.control_event(control::Event::Setup(packet), rate);
            }
            Event::OutCompleted {
                entry: OutEntry::Control,
                untransferred,
            } => self.control_data_received(untransferred, rate),
            Event::InCompleted(InEntry::Control) => {
                if self.control_in.complete() {
                    self.control_event(control::Event::InSent, rate);
                }
            }
            Event::BusReset(speed) => {
                self.reaction.commands.push(Command::ResetEndpoints);
                self.reaction
                    .commands
                    .push(Command::ApplyAddress(Address::Default));
                self.reclaim_every_buffer();
                self.control_event(control::Event::BusReset, rate);
                self.function_event(function::Event::BusReset(speed));
            }
            Event::Detached => {
                self.reaction.commands.push(Command::ResetEndpoints);
                self.reclaim_every_buffer();
                self.control_event(control::Event::BusReset, rate);
                self.function_event(function::Event::Detached);
            }
            Event::Attached => self.function_event(function::Event::Attached),
            Event::Suspended => self.function_event(function::Event::Suspended),
            Event::Resumed => self.function_event(function::Event::Resumed),
        }
        &self.reaction
    }

    #[inline(never)]
    fn control_data_received(&mut self, untransferred: ByteCount, rate: &rate_change::ClockReport) {
        let mut packet = [0; CONTROL_PACKET_BYTES];
        let mut length = 0;
        let landed = self.control_out.complete(untransferred, |received| {
            for (slot, byte) in packet.iter_mut().zip(received.bytes()) {
                *slot = byte;
                length += 1;
            }
        });
        if landed {
            let data = packet.get(..length).unwrap_or(&[]);
            self.control_event(control::Event::OutReceived(data), rate);
        }
    }

    fn reclaim_every_buffer(&mut self) {
        self.control_out.reclaim();
        self.control_in.reclaim();
        self.data.reclaim_after_reset();
    }

    #[inline(never)]
    fn control_event(&mut self, event: control::Event<'_>, rate: &rate_change::ClockReport) {
        let context = ControlContext {
            function: self.function,
            rate: *rate,
            line_coding: self.line_coding,
            descriptors: self.descriptors,
        };
        let (next, outputs) = control::transition(self.control, event, &context);
        self.control = next;
        if let (
            control::Event::Setup(_),
            control::State::ConfirmingWrite(Effect::Function(function::Event::AddressSet(address))),
        ) = (event, next)
        {
            self.reaction.commands.push(Command::ApplyAddress(address));
        }
        for output in outputs.as_slice() {
            match *output {
                control::Output::ReleaseStall => {
                    self.reaction.commands.push(Command::ReleaseControlStall);
                }
                control::Output::Stall => {
                    self.control_out.retire(&mut self.reaction.commands);
                    self.control_in.retire(&mut self.reaction.commands);
                    self.reaction.commands.push(Command::StallControl);
                }
                control::Output::Send(packet) => self.control_in.send_if_held(
                    |buffer| Some(buffer.fill_bytes(packet.bytes().iter().copied())),
                    &mut self.reaction.commands,
                ),
                control::Output::Receive => {
                    self.control_out
                        .arm(CONTROL_ROOM, &mut self.reaction.commands);
                }
                control::Output::Effect(effect) => self.apply(effect),
            }
        }
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::Function(event) => self.function_event(event),
            Effect::RateChange(event) => self.reaction.notices.push(Notice::RateChange(event)),
            Effect::LineCoding(coding) => self.line_coding = coding,
            Effect::DataTerminal(terminal) => {
                self.reaction.notices.push(Notice::DataTerminal(terminal));
            }
        }
    }

    #[inline(never)]
    fn function_event(&mut self, event: function::Event) {
        let serial_before = serial_speed(self.function);
        let (next, outputs) = function::transition(self.function, event);
        self.function = next;
        let Reaction { commands, notices } = &mut self.reaction;
        for output in outputs.as_slice() {
            match *output {
                function::Output::SelectPlayback(setting) => {
                    self.streams.playback.select_alternate(setting);
                    self.data.playback_selected(setting, commands);
                }
                function::Output::SelectCapture(setting) => {
                    self.streams.capture.select_alternate(setting);
                    self.data.capture_selected(setting, commands);
                }
                function::Output::HaltEndpoint(endpoint) => commands.push(Command::Halt(endpoint)),
                function::Output::ClearEndpointHalt(endpoint) => {
                    commands.push(Command::ReleaseHalt(endpoint));
                }
                function::Output::FadeOutputsToSilence => {
                    notices.push(Notice::FadeOutputsToSilence);
                }
                function::Output::ReleaseOutputSilence => {
                    notices.push(Notice::ReleaseOutputSilence);
                }
                function::Output::SuspendPhantom => notices.push(Notice::SuspendPhantom),
                function::Output::ResumePhantom => notices.push(Notice::ResumePhantom),
            }
        }
        match (serial_before, serial_speed(next)) {
            (None, Some(speed)) => self.data.serial_opened(speed, commands),
            (Some(_), None) => self.data.serial_closed(commands),
            (None, None) | (Some(_), Some(_)) => {}
        }
    }
}
