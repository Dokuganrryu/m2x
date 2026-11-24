use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Nanoseconds(pub u64);

impl Nanoseconds {
    pub const fn from_milliseconds(milliseconds: u64) -> Self {
        Self(milliseconds * 1_000_000)
    }

    pub const fn since(self, earlier: Nanoseconds) -> Nanoseconds {
        Nanoseconds(self.0.saturating_sub(earlier.0))
    }
}

impl fmt::Display for Nanoseconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3} ms", self.0 as f64 / 1e6)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    High,
    Low,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I2cStatus {
    Acked,
    NackAddress,
    NackData(usize),
    Hung,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum I2cTransaction {
    Write {
        address: u8,
        bytes: Vec<u8>,
        status: I2cStatus,
    },
    Read {
        address: u8,
        register: u8,
        value: u8,
        status: I2cStatus,
    },
    AddressOnly {
        address: u8,
        status: I2cStatus,
    },
}

impl I2cTransaction {
    pub const fn address(&self) -> u8 {
        match self {
            I2cTransaction::Write { address, .. }
            | I2cTransaction::Read { address, .. }
            | I2cTransaction::AddressOnly { address, .. } => *address,
        }
    }

    pub const fn status(&self) -> I2cStatus {
        match self {
            I2cTransaction::Write { status, .. }
            | I2cTransaction::Read { status, .. }
            | I2cTransaction::AddressOnly { status, .. } => *status,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Fetch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    TimeLimit,
    ResetRequest,
    WatchdogReset,
    Stopped(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Behavior {
        id: String,
        label: String,
        text: String,
    },
    Write {
        register: String,
        value: u32,
    },
    Read {
        register: String,
    },
    Pin {
        port: u8,
        number: u8,
        level: Level,
    },
    PreampWord {
        bits: u32,
        value: u32,
    },
    SpiCut,
    I2c(I2cTransaction),
    MainClock {
        hertz: u32,
    },
    MpuEnabled,
    MpuDisabled,
    MpuFault {
        address: u32,
        access: Access,
    },
    MpuProbe {
        address: u32,
        access: Access,
        allowed: bool,
    },
    FlashControllerAccess {
        address: u32,
    },
    SecureAliasAccess {
        address: u32,
    },
    FlashRead {
        address: u32,
    },
    FlashWrite {
        address: u32,
    },
    WatchdogArmed,
    WatchdogFed,
    WatchdogStarved,
    WatchdogBadFeed,
    UsbConnected,
    UsbDisconnected,
    BusFault {
        address: u32,
        ignored: bool,
    },
    Exception {
        number: u32,
    },
    BreadcrumbMagic(u32),
    Breadcrumb(u32),
    HandoverWrite {
        address: u32,
        value: u32,
    },
    PlantedStore {
        address: u32,
    },
    InterruptsDelivered {
        irq: u32,
        count: u32,
    },
    InterruptsDeliveredAtBreadcrumb {
        irq: u32,
        count: u32,
    },
    ResetRequest,
    StoppedAt(u32),
    End(Ending),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub at: Nanoseconds,
    pub event: Event,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub entries: Vec<Entry>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TraceError {
    pub line: usize,
    pub text: String,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the board model printed line {} that the gate does not read: `{}`",
            self.line, self.text
        )
    }
}

impl Trace {
    pub fn parse(text: &str) -> Result<Self, TraceError> {
        let mut entries = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let refuse = || TraceError {
                line: index + 1,
                text: line.to_string(),
            };
            let (at, rest) = line.split_once(' ').ok_or_else(refuse)?;
            let at = Nanoseconds(at.parse().map_err(|_| refuse())?);
            let event = event(rest).ok_or_else(refuse)?;
            entries.push(Entry { at, event });
        }
        entries.sort_by_key(|entry| entry.at);
        Ok(Self { entries })
    }

    pub fn guesses(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| match &entry.event {
                Event::Behavior { id, label, text } if label == "guess" => {
                    Some(format!("{id}: {text}"))
                }
                _ => None,
            })
            .collect()
    }

    pub fn ending(&self) -> Option<&Ending> {
        self.entries.iter().find_map(|entry| match &entry.event {
            Event::End(ending) => Some(ending),
            _ => None,
        })
    }
}

fn number(word: &str) -> Option<u32> {
    match word.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => word.parse().ok(),
    }
}

fn byte(word: &str) -> Option<u8> {
    number(word).and_then(|value| u8::try_from(value).ok())
}

fn field<'a>(word: &'a str, name: &str) -> Option<&'a str> {
    word.strip_prefix(name)?.strip_prefix('=')
}

fn access(word: &str) -> Option<Access> {
    match word {
        "read" => Some(Access::Read),
        "write" => Some(Access::Write),
        "fetch" => Some(Access::Fetch),
        _ => None,
    }
}

fn status(word: &str) -> Option<I2cStatus> {
    match word {
        "ack" => Some(I2cStatus::Acked),
        "nack-address" => Some(I2cStatus::NackAddress),
        "hung" => Some(I2cStatus::Hung),
        _ if word.starts_with("abandoned") => Some(I2cStatus::Abandoned),
        _ => word
            .strip_prefix("nack-data@")
            .and_then(|index| index.parse().ok())
            .map(I2cStatus::NackData),
    }
}

fn pin(name: &str, level: &str) -> Option<Event> {
    let (port, number) = name.strip_prefix('P')?.split_once('_')?;
    let level = match level {
        "high" => Level::High,
        "low" => Level::Low,
        "released" => Level::Released,
        _ => return None,
    };
    Some(Event::Pin {
        port: port.parse().ok()?,
        number: number.parse().ok()?,
        level,
    })
}

fn i2c(words: &[&str]) -> Option<Event> {
    let transaction = match words {
        ["write", address, bytes, state] => I2cTransaction::Write {
            address: byte(address)?,
            bytes: bytes.split(',').map(byte).collect::<Option<Vec<u8>>>()?,
            status: status(state)?,
        },
        ["read", address, "reg", register, "value", value, state] => I2cTransaction::Read {
            address: byte(address)?,
            register: byte(register)?,
            value: byte(value)?,
            status: status(state)?,
        },
        ["address", address, state] => I2cTransaction::AddressOnly {
            address: byte(address)?,
            status: status(state)?,
        },
        _ => return None,
    };
    Some(Event::I2c(transaction))
}

fn ending(words: &[&str]) -> Option<Ending> {
    match words {
        ["time-limit"] => Some(Ending::TimeLimit),
        ["reset-request"] => Some(Ending::ResetRequest),
        ["watchdog-reset"] => Some(Ending::WatchdogReset),
        [reason] if reason.starts_with("stopped") => Some(Ending::Stopped(reason.to_string())),
        _ => None,
    }
}

fn event(rest: &str) -> Option<Event> {
    if let Some(behavior) = rest.strip_prefix("behavior ") {
        let (head, text) = behavior.split_once(" | ")?;
        let (id, label) = head.split_once(' ')?;
        return Some(Event::Behavior {
            id: id.to_string(),
            label: label.to_string(),
            text: text.to_string(),
        });
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    Some(match words.as_slice() {
        ["write", register, value] => Event::Write {
            register: register.to_string(),
            value: number(value)?,
        },
        ["read", register] => Event::Read {
            register: register.to_string(),
        },
        ["pin", name, level] => pin(name, level)?,
        ["preamp-word", bits, value] => Event::PreampWord {
            bits: number(field(bits, "bits")?)?,
            value: number(field(value, "value")?)?,
        },
        ["spi-cut", _] => Event::SpiCut,
        ["i2c", rest @ ..] => i2c(rest)?,
        ["clock", "main", hertz] => Event::MainClock {
            hertz: number(hertz)?,
        },
        ["mpu", "enable", ..] => Event::MpuEnabled,
        ["mpu", "disable"] => Event::MpuDisabled,
        ["mpu-fault", address, kind, _] => Event::MpuFault {
            address: number(address)?,
            access: access(kind)?,
        },
        ["mpu-probe", address, kind, verdict] => Event::MpuProbe {
            address: number(address)?,
            access: access(kind)?,
            allowed: match *verdict {
                "allowed" => true,
                "fault" => false,
                _ => return None,
            },
        },
        ["flash-controller", _, address, ..] => Event::FlashControllerAccess {
            address: number(address)?,
        },
        ["secure-alias", _, address, ..] => Event::SecureAliasAccess {
            address: number(address)?,
        },
        ["flash-read", address] => Event::FlashRead {
            address: number(address)?,
        },
        ["flash-write", address, _] => Event::FlashWrite {
            address: number(address)?,
        },
        ["watchdog", "arm", ..] => Event::WatchdogArmed,
        ["watchdog", "feed"] => Event::WatchdogFed,
        ["watchdog", "starved"] => Event::WatchdogStarved,
        ["watchdog", "bad-feed", ..] => Event::WatchdogBadFeed,
        ["usb", "connect"] => Event::UsbConnected,
        ["usb", "disconnect"] => Event::UsbDisconnected,
        ["bus-fault", "ignored", address] => Event::BusFault {
            address: number(address)?,
            ignored: true,
        },
        ["bus-fault", address] => Event::BusFault {
            address: number(address)?,
            ignored: false,
        },
        ["exception", exception, "from", _] => Event::Exception {
            number: number(exception)?,
        },
        ["breadcrumb-magic", word] => Event::BreadcrumbMagic(number(word)?),
        ["breadcrumb", word] => Event::Breadcrumb(number(word)?),
        ["handover-write", address, value] => Event::HandoverWrite {
            address: number(address)?,
            value: number(value)?,
        },
        ["planted-store", address] => Event::PlantedStore {
            address: number(address)?,
        },
        ["irq", irq, "delivered", count] => Event::InterruptsDelivered {
            irq: number(irq)?,
            count: number(count)?,
        },
        ["irq", irq, "delivered", count, "at-breadcrumb"] => {
            Event::InterruptsDeliveredAtBreadcrumb {
                irq: number(irq)?,
                count: number(count)?,
            }
        }
        ["reset-request"] => Event::ResetRequest,
        ["stopped-at", pc] => Event::StoppedAt(number(pc)?),
        ["end", reason @ ..] => Event::End(ending(reason)?),
        _ => return None,
    })
}
