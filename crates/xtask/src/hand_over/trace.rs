use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Startup,
    Frame(u8),
    Exception(u8),
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Write {
    pub index: u64,
    pub pc: u32,
    pub address: u32,
    pub size: u32,
    pub value: u32,
    pub context: Context,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandOverState {
    pub primask: u32,
    pub iser: [u32; 2],
    pub ispr: [u32; 2],
    pub icsr: u32,
    pub vtor: u32,
    pub syst_csr: u32,
    pub dma0_ctrl: u32,
    pub dma0_enabled: u32,
    pub ctimer0_tcr: u32,
    pub usb_devcmdstat: u32,
    pub usb1_dev_clock: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Write(Write),
    Exception {
        number: u8,
        handler: u32,
        index: u64,
    },
    Returned {
        number: u8,
        index: u64,
    },
    NoReturn {
        number: u8,
        pc: u32,
    },
    Reached {
        pc: u32,
        index: u64,
    },
    Stopped {
        pc: u32,
        index: u64,
        reason: String,
    },
    State(HandOverState),
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
            "the emulator printed a line the gate cannot read, line {}: `{}`",
            self.line, self.text
        )
    }
}

fn hex(word: &str) -> Option<u32> {
    u32::from_str_radix(word, 16).ok()
}

fn context(word: &str) -> Option<Context> {
    match word {
        "startup" => Some(Context::Startup),
        "after" => Some(Context::After),
        _ => {
            if let Some(number) = word.strip_prefix("frame") {
                number.parse().ok().map(Context::Frame)
            } else {
                word.strip_prefix("exception")?
                    .parse()
                    .ok()
                    .map(Context::Exception)
            }
        }
    }
}

fn state(fields: &[&str]) -> Option<HandOverState> {
    let mut state = HandOverState::default();
    for field in fields {
        let (name, value) = field.split_once('=')?;
        let value = hex(value)?;
        let slot = match name {
            "primask" => &mut state.primask,
            "iser0" => &mut state.iser[0],
            "iser1" => &mut state.iser[1],
            "ispr0" => &mut state.ispr[0],
            "ispr1" => &mut state.ispr[1],
            "icsr" => &mut state.icsr,
            "vtor" => &mut state.vtor,
            "syst_csr" => &mut state.syst_csr,
            "dma0_ctrl" => &mut state.dma0_ctrl,
            "dma0_enabled" => &mut state.dma0_enabled,
            "ctimer0_tcr" => &mut state.ctimer0_tcr,
            "usb_devcmdstat" => &mut state.usb_devcmdstat,
            "usb1_dev_clock" => &mut state.usb1_dev_clock,
            _ => return None,
        };
        *slot = value;
    }
    Some(state)
}

fn event(line: &str) -> Option<Event> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["write", index, pc, address, size, value, at] => Some(Event::Write(Write {
            index: index.parse().ok()?,
            pc: hex(pc)?,
            address: hex(address)?,
            size: size.parse().ok()?,
            value: hex(value)?,
            context: context(at)?,
        })),
        ["exception", number, handler, index] => Some(Event::Exception {
            number: number.parse().ok()?,
            handler: hex(handler)?,
            index: index.parse().ok()?,
        }),
        ["return", number, index] => Some(Event::Returned {
            number: number.parse().ok()?,
            index: index.parse().ok()?,
        }),
        ["noreturn", number, pc] => Some(Event::NoReturn {
            number: number.parse().ok()?,
            pc: hex(pc)?,
        }),
        ["reached", pc, index] => Some(Event::Reached {
            pc: hex(pc)?,
            index: index.parse().ok()?,
        }),
        ["stopped", pc, index, reason @ ..] => Some(Event::Stopped {
            pc: hex(pc)?,
            index: index.parse().ok()?,
            reason: reason.join(" "),
        }),
        ["state", fields @ ..] => state(fields).map(Event::State),
        _ => None,
    }
}

pub fn read(output: &str) -> Result<Vec<Event>, TraceError> {
    output
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(number, line)| {
            event(line).ok_or_else(|| TraceError {
                line: number + 1,
                text: line.to_string(),
            })
        })
        .collect()
}
