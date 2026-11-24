use super::command::ConsoleCommand;
use super::line::{Appended, Line};
use super::refusal::Refusal;
use crate::outputs::{Filler, OutputList};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Closed,
    Collecting(Line),
    Discarding(DiscardCause),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardCause {
    LineTooLong,
    NotPrintableAscii,
}

impl DiscardCause {
    const fn refusal(self) -> Refusal {
        match self {
            Self::LineTooLong => Refusal::LineTooLong,
            Self::NotPrintableAscii => Refusal::NotPrintableAscii,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    PortOpened,
    PortClosed,
    Byte(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Output {
    Execute(ConsoleCommand),
    Reject(Refusal),
}

impl Output {
    pub const fn admitted(self, running: bool) -> Self {
        match self {
            Self::Execute(command) if !running && !command.only_reads() => {
                Self::Reject(Refusal::NotRunning)
            }
            Self::Execute(_) | Self::Reject(_) => self,
        }
    }
}

impl Filler for Output {
    const FILLER: Self = Output::Reject(Refusal::NotAllowlisted);
}

pub type Outputs = OutputList<Output, 1>;

enum ByteClass {
    LineEnd,
    Text(u8),
    NotText,
}

const CARRIAGE_RETURN: u8 = b'\r';
const LINE_FEED: u8 = b'\n';
const PRINTABLE_ASCII: core::ops::RangeInclusive<u8> = b' '..=b'~';

impl ByteClass {
    fn of(byte: u8) -> Self {
        if byte == CARRIAGE_RETURN || byte == LINE_FEED {
            Self::LineEnd
        } else if PRINTABLE_ASCII.contains(&byte) {
            Self::Text(byte)
        } else {
            Self::NotText
        }
    }
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    match (state, event) {
        (State::Closed | State::Collecting(_) | State::Discarding(_), Event::PortClosed)
        | (State::Closed, Event::Byte(_)) => (State::Closed, Outputs::NONE),
        (State::Closed | State::Collecting(_) | State::Discarding(_), Event::PortOpened) => {
            (State::Collecting(Line::EMPTY), Outputs::NONE)
        }
        (State::Collecting(line), Event::Byte(byte)) => collect(line, ByteClass::of(byte)),
        (State::Discarding(cause), Event::Byte(byte)) => discard(cause, ByteClass::of(byte)),
    }
}

fn collect(line: Line, byte: ByteClass) -> (State, Outputs) {
    match byte {
        ByteClass::LineEnd => (State::Collecting(Line::EMPTY), finished(&line)),
        ByteClass::Text(text) => match line.appended(text) {
            Appended::Stored(longer) => (State::Collecting(longer), Outputs::NONE),
            Appended::Overflowed => (State::Discarding(DiscardCause::LineTooLong), Outputs::NONE),
        },
        ByteClass::NotText => (
            State::Discarding(DiscardCause::NotPrintableAscii),
            Outputs::NONE,
        ),
    }
}

fn discard(cause: DiscardCause, byte: ByteClass) -> (State, Outputs) {
    match byte {
        ByteClass::LineEnd => (
            State::Collecting(Line::EMPTY),
            Outputs::of([Output::Reject(cause.refusal())]),
        ),
        ByteClass::Text(_) | ByteClass::NotText => (State::Discarding(cause), Outputs::NONE),
    }
}

fn finished(line: &Line) -> Outputs {
    if line.words().next().is_none() {
        return Outputs::NONE;
    }
    Outputs::of([match ConsoleCommand::parse(line) {
        Ok(command) => Output::Execute(command),
        Err(refusal) => Output::Reject(refusal),
    }])
}
