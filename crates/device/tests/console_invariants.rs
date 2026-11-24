use device::console::{
    ConsoleCommand, Event, LINE_CAPACITY, Output, Refusal, Report, State, transition,
};
use dsp::{
    Band, BandFrequency, BandGain, BandQ, Ceiling, DspMode, FilterKind, LimiterSettings,
    ReleaseTime,
};

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;

struct XorShift(u64);

impl XorShift {
    fn seeded(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<'a>(&mut self, choices: &[&'a str]) -> &'a str {
        choices[self.below(choices.len())]
    }
}

const ALLOWLISTED: &[&str] = &[
    "status",
    "counters",
    "dma",
    "knob",
    "serial",
    "version",
    "cycles",
    "fault",
    "panel",
    "clocks",
    "i2c",
    "power",
    "dsp bypass",
    "dsp on",
    "meter reset",
    "peq off 1",
    "peq off 5",
    "peq 1 peak 1000 6 1",
    "peq 3 lowshelf 20 -12 0.3",
    "peq 5 highpass 20000 12 10",
    "lim -1 50",
    "lim -20 1000",
    "lim 0 5",
    "  status  ",
];

const NEAR_MISSES: &[&str] = &[
    "i2cset 0x48 0 0",
    "i2cget 0x48 113",
    "adc on",
    "phantom on",
    "gain 1 60",
    "volume up",
    "reboot",
    "bootloader",
    "dfu",
    "flash",
    "erase 0x38200",
    "q",
    "r",
    "peq 6 peak 1000 0 1",
    "peq 1 peak 1000 13 1",
    "peq 1 peak 1e3 0 1",
    "peq 1 bell 1000 0 1",
    "peq off",
    "lim -21 50",
    "lim -1 4",
    "lim -1.25 50",
    "dsp off",
    "meter",
    "status now",
    "STATUS",
];

const WORDS: &[&str] = &[
    "peq",
    "lim",
    "dsp",
    "on",
    "off",
    "bypass",
    "meter",
    "reset",
    "status",
    "peak",
    "highshelf",
    "lowpass",
    "1",
    "5",
    "-1",
    "50",
    "1000",
    "0.3",
    "12",
    "-12.0",
    "20000",
    "+3",
    "nan",
];

#[derive(Clone, Copy)]
enum Diet {
    Commands,
    RawBytes,
}

fn random_band_command(random: &mut XorShift) -> String {
    let kind = random.pick(&[
        "peak",
        "lowshelf",
        "highshelf",
        "highpass",
        "lowpass",
        "notch",
    ]);
    format!(
        "peq {} {kind} {}.{} {}{}.{} {}.{}",
        random.below(7),
        random.below(22_000),
        random.below(10),
        random.pick(&["", "-", "+"]),
        random.below(14),
        random.below(100),
        random.below(11),
        random.below(100),
    )
}

fn random_limiter_command(random: &mut XorShift) -> String {
    format!(
        "lim {}{}.{} {}",
        random.pick(&["", "-", "+"]),
        random.below(23),
        random.below(10),
        random.below(1_100)
    )
}

fn framed(random: &mut XorShift, text: &str) -> Vec<Event> {
    let ending = random.pick(&["\n", "\r", "\r\n", "", ""]);
    let opening = random.pick(&["\n", "", "", ""]);
    format!("{opening}{text}{ending}")
        .bytes()
        .map(Event::Byte)
        .collect()
}

fn command_fragment(random: &mut XorShift) -> Vec<Event> {
    match random.below(100) {
        0..=29 => {
            let text = random.pick(ALLOWLISTED);
            framed(random, text)
        }
        30..=44 => {
            let text = random.pick(NEAR_MISSES);
            framed(random, text)
        }
        45..=52 => {
            let text = random_band_command(random);
            framed(random, &text)
        }
        53..=57 => {
            let text = random_limiter_command(random);
            framed(random, &text)
        }
        58..=67 => random.pick(WORDS).bytes().map(Event::Byte).collect(),
        68..=71 => vec![Event::Byte(b' ')],
        72..=86 => random
            .pick(&["\n", "\r", "\r\n"])
            .bytes()
            .map(Event::Byte)
            .collect(),
        87..=89 => vec![Event::Byte(random.below(256) as u8)],
        90..=93 => vec![Event::Byte(b'x'); random.below(2 * LINE_CAPACITY)],
        94..=95 => vec![Event::Byte((0x20 + random.below(0x5F)) as u8)],
        96..=97 => vec![Event::PortOpened],
        _ => vec![Event::PortClosed],
    }
}

fn raw_byte_fragment(random: &mut XorShift) -> Vec<Event> {
    match random.below(1_000) {
        0..=4 => vec![Event::PortOpened],
        5..=7 => vec![Event::PortClosed],
        8..=99 => vec![Event::Byte(*b"\r\n".get(random.below(2)).unwrap())],
        _ => vec![Event::Byte(random.below(256) as u8)],
    }
}

fn sequence(seed: u64, diet: Diet) -> Vec<Event> {
    let mut random = XorShift::seeded(seed);
    let mut events = vec![Event::PortOpened];
    while events.len() < EVENTS_PER_SEQUENCE {
        events.extend(match diet {
            Diet::Commands => command_fragment(&mut random),
            Diet::RawBytes => raw_byte_fragment(&mut random),
        });
    }
    events.truncate(EVENTS_PER_SEQUENCE);
    events
}

fn plain_decimal(word: &str) -> Option<(&str, Option<&str>)> {
    let unsigned = word.strip_prefix(['+', '-']).unwrap_or(word);
    let mut parts = unsigned.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    let digits = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    (parts.next().is_none() && digits(whole) && fraction.is_none_or(digits))
        .then_some((whole, fraction))
}

fn reference_band(words: &[&str]) -> Option<Band> {
    let [kind, hz, db, q] = words else {
        return None;
    };
    let kind = match *kind {
        "peak" => FilterKind::Peaking,
        "lowshelf" => FilterKind::LowShelf,
        "highshelf" => FilterKind::HighShelf,
        "highpass" => FilterKind::HighPass,
        "lowpass" => FilterKind::LowPass,
        _ => return None,
    };
    let number = |word: &str| plain_decimal(word).and_then(|_| word.parse::<f64>().ok());
    Some(Band {
        kind,
        frequency: BandFrequency::from_hz(number(hz)?).ok()?,
        gain: BandGain::from_db(number(db)?).ok()?,
        q: BandQ::new(number(q)?).ok()?,
    })
}

fn reference_band_index(word: &str) -> Option<usize> {
    let digits_only = !word.is_empty() && word.chars().all(|c| c.is_ascii_digit());
    let number: u64 = digits_only.then(|| word.parse().ok()).flatten()?;
    (1..=dsp::PEQ_BANDS as u64)
        .contains(&number)
        .then(|| number as usize - 1)
}

fn reference_limiter(words: &[&str]) -> Option<LimiterSettings> {
    let [ceiling, release] = words else {
        return None;
    };
    let (_, fraction) = plain_decimal(ceiling)?;
    if fraction.is_some_and(|digits| digits.len() > 1) {
        return None;
    }
    let tenths = (ceiling.parse::<f64>().ok()? * 10.0).round();
    let release_digits = !release.is_empty() && release.chars().all(|c| c.is_ascii_digit());
    let millis: u64 = release_digits.then(|| release.parse().ok()).flatten()?;
    Some(LimiterSettings {
        ceiling: Ceiling::from_tenths_of_dbfs(i16::try_from(tenths as i64).ok()?).ok()?,
        release: ReleaseTime::from_millis(u16::try_from(millis).ok()?).ok()?,
    })
}

#[derive(Debug, PartialEq)]
enum Allowlisted {
    Exactly(ConsoleCommand),
    DisableBand(usize),
    SetBand(usize, Band),
}

fn reference_command(line: &[u8]) -> Option<Allowlisted> {
    let text = std::str::from_utf8(line).ok()?;
    let words: Vec<&str> = text.split(' ').filter(|word| !word.is_empty()).collect();
    let exactly = |command| Some(Allowlisted::Exactly(command));
    match words.as_slice() {
        ["status"] => exactly(ConsoleCommand::Report(Report::Status)),
        ["counters"] => exactly(ConsoleCommand::Report(Report::Counters)),
        ["dma"] => exactly(ConsoleCommand::Report(Report::Dma)),
        ["knob"] => exactly(ConsoleCommand::Report(Report::Knob)),
        ["serial"] => exactly(ConsoleCommand::Report(Report::Serial)),
        ["version"] => exactly(ConsoleCommand::Report(Report::Version)),
        ["cycles"] => exactly(ConsoleCommand::Report(Report::Cycles)),
        ["fault"] => exactly(ConsoleCommand::Report(Report::Fault)),
        ["panel"] => exactly(ConsoleCommand::Report(Report::Panel)),
        ["clocks"] => exactly(ConsoleCommand::Report(Report::Clocks)),
        ["i2c"] => exactly(ConsoleCommand::Report(Report::Bus)),
        ["power"] => exactly(ConsoleCommand::Report(Report::Power)),
        ["dsp", "bypass"] => exactly(ConsoleCommand::SetDspMode(DspMode::Bypass)),
        ["dsp", "on"] => exactly(ConsoleCommand::SetDspMode(DspMode::On)),
        ["meter", "reset"] => exactly(ConsoleCommand::ResetMeters),
        #[cfg(feature = "sound3")]
        ["meter"] => exactly(ConsoleCommand::MeterProfile(None)),
        #[cfg(feature = "sound3")]
        ["meter", word] => exactly(ConsoleCommand::MeterProfile(Some(
            device::lcd::screen::MeterProfileName::named(word.as_bytes())?,
        ))),
        #[cfg(feature = "sound3")]
        ["power", "nmpa"] => exactly(ConsoleCommand::Report(Report::PowerNmpa)),
        ["lim", rest @ ..] => exactly(ConsoleCommand::SetLimiter(reference_limiter(rest)?)),
        ["peq", "off", number] => Some(Allowlisted::DisableBand(reference_band_index(number)?)),
        ["peq", number, rest @ ..] => Some(Allowlisted::SetBand(
            reference_band_index(number)?,
            reference_band(rest)?,
        )),
        _ => None,
    }
}

fn as_allowlisted(command: ConsoleCommand) -> Allowlisted {
    match command {
        ConsoleCommand::DisableBand(band) => Allowlisted::DisableBand(band.index()),
        ConsoleCommand::SetBand { band, settings } => Allowlisted::SetBand(band.index(), settings),
        ConsoleCommand::Report(_)
        | ConsoleCommand::SetDspMode(_)
        | ConsoleCommand::SetLimiter(_)
        | ConsoleCommand::ResetMeters => Allowlisted::Exactly(command),
        #[cfg(feature = "register-zero-protocol")]
        ConsoleCommand::RegisterZero(_) => Allowlisted::Exactly(command),
        #[cfg(feature = "sound2")]
        ConsoleCommand::Preset(_)
        | ConsoleCommand::SetTrim(_)
        | ConsoleCommand::BeginBands
        | ConsoleCommand::CommitBands => Allowlisted::Exactly(command),
        #[cfg(feature = "screen2")]
        ConsoleCommand::LcdMotion(_) => Allowlisted::Exactly(command),
        #[cfg(feature = "sound3")]
        ConsoleCommand::MeterProfile(_) => Allowlisted::Exactly(command),
    }
}

enum Discard {
    Clean,
    Because(Refusal),
}

struct Model {
    open: bool,
    line: Vec<u8>,
    discard: Discard,
}

#[derive(Default)]
struct Tally {
    executed: usize,
    too_long: usize,
    not_ascii: usize,
    other_refusals: usize,
    port_drops_of_partial_lines: usize,
}

enum Expected {
    Nothing,
    Exactly(Output),
    LineVerdict(Vec<u8>),
}

impl Model {
    fn closed() -> Self {
        Self {
            open: false,
            line: Vec::new(),
            discard: Discard::Clean,
        }
    }

    fn expect(&mut self, event: Event, tally: &mut Tally) -> Expected {
        match event {
            Event::PortOpened | Event::PortClosed => {
                if !self.line.is_empty() || !matches!(self.discard, Discard::Clean) {
                    tally.port_drops_of_partial_lines += 1;
                }
                self.open = matches!(event, Event::PortOpened);
                self.line.clear();
                self.discard = Discard::Clean;
                Expected::Nothing
            }
            Event::Byte(_) if !self.open => Expected::Nothing,
            Event::Byte(b'\n' | b'\r') => {
                let line = std::mem::take(&mut self.line);
                match std::mem::replace(&mut self.discard, Discard::Clean) {
                    Discard::Because(refusal) => Expected::Exactly(Output::Reject(refusal)),
                    Discard::Clean if line.iter().all(|&byte| byte == b' ') => Expected::Nothing,
                    Discard::Clean => Expected::LineVerdict(line),
                }
            }
            Event::Byte(byte) => {
                if matches!(self.discard, Discard::Clean) {
                    if !(b' '..=b'~').contains(&byte) {
                        self.discard = Discard::Because(Refusal::NotPrintableAscii);
                    } else if self.line.len() == LINE_CAPACITY {
                        self.discard = Discard::Because(Refusal::LineTooLong);
                    } else {
                        self.line.push(byte);
                    }
                }
                Expected::Nothing
            }
        }
    }
}

fn check(seed: u64, diet: Diet, tally: &mut Tally) {
    let mut state = State::Closed;
    let mut model = Model::closed();
    for (step, event) in sequence(seed, diet).into_iter().enumerate() {
        let (next, outputs) = transition(state, event);
        let outputs = outputs.as_slice();
        let context = format!("seed {seed} step {step} event {event:?} gave {outputs:?}");
        match model.expect(event, tally) {
            Expected::Nothing => assert_eq!(outputs, [], "{context}"),
            Expected::Exactly(output) => {
                assert_eq!(outputs, [output], "{context}");
                match output {
                    Output::Reject(Refusal::LineTooLong) => tally.too_long += 1,
                    Output::Reject(Refusal::NotPrintableAscii) => tally.not_ascii += 1,
                    Output::Reject(_) | Output::Execute(_) => {
                        panic!("{context}: a discard ended in {output:?}")
                    }
                }
            }
            Expected::LineVerdict(line) => {
                let shown = String::from_utf8_lossy(&line).into_owned();
                match (outputs, reference_command(&line)) {
                    ([Output::Execute(command)], Some(reference)) => {
                        assert_eq!(
                            as_allowlisted(*command),
                            reference,
                            "{context} line {shown:?}"
                        );
                        tally.executed += 1;
                    }
                    (
                        [
                            Output::Reject(
                                Refusal::NotAllowlisted
                                | Refusal::MalformedArguments
                                | Refusal::SettingOutOfRange,
                            ),
                        ],
                        None,
                    ) => tally.other_refusals += 1,
                    _ => panic!("{context} line {shown:?}"),
                }
            }
        }
        state = next;
    }
}

#[test]
fn only_complete_allowlisted_lines_execute_and_every_discarded_line_is_refused_once() {
    let mut tally = Tally::default();
    for seed in 0..SEQUENCES {
        check(seed, Diet::Commands, &mut tally);
    }
    assert!(tally.executed > 50_000, "executed {}", tally.executed);
    assert!(tally.too_long > 5_000, "too long {}", tally.too_long);
    assert!(tally.not_ascii > 4_000, "not ascii {}", tally.not_ascii);
    assert!(
        tally.other_refusals > 75_000,
        "refused {}",
        tally.other_refusals
    );
    assert!(
        tally.port_drops_of_partial_lines > 5_000,
        "port drops {}",
        tally.port_drops_of_partial_lines
    );
}

#[test]
fn random_bytes_never_panic_and_never_execute_a_line_outside_the_allowlist() {
    let mut tally = Tally::default();
    for seed in 0..SEQUENCES {
        check(seed, Diet::RawBytes, &mut tally);
    }
    assert!(tally.not_ascii > 250_000, "not ascii {}", tally.not_ascii);
    assert!(
        tally.other_refusals > 15_000,
        "refused {}",
        tally.other_refusals
    );
}
