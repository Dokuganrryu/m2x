#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdcAddress {
    At13,
    At4F,
}

impl AdcAddress {
    pub const fn address(self) -> u8 {
        match self {
            AdcAddress::At13 => 0x13,
            AdcAddress::At4F => 0x4F,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeReply {
    Acked,
    Nacked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdcKind {
    Unanswered,
    Answered(AdcAddress),
}

impl AdcKind {
    pub const fn from_probes(at_13: ProbeReply, at_4f: ProbeReply) -> Self {
        match (at_13, at_4f) {
            (ProbeReply::Acked, ProbeReply::Acked | ProbeReply::Nacked) => {
                AdcKind::Answered(AdcAddress::At13)
            }
            (ProbeReply::Nacked, ProbeReply::Acked) => AdcKind::Answered(AdcAddress::At4F),
            (ProbeReply::Nacked, ProbeReply::Nacked) => AdcKind::Unanswered,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdcWrite {
    At13Register0,
    At13Register1,
    At13Register2,
    At13Register4,
    At4FRegister1,
}

impl AdcWrite {
    pub const fn address(self) -> AdcAddress {
        match self {
            AdcWrite::At13Register0
            | AdcWrite::At13Register1
            | AdcWrite::At13Register2
            | AdcWrite::At13Register4 => AdcAddress::At13,
            AdcWrite::At4FRegister1 => AdcAddress::At4F,
        }
    }

    pub const fn register(self) -> u8 {
        match self {
            AdcWrite::At13Register0 => 0,
            AdcWrite::At13Register1 | AdcWrite::At4FRegister1 => 1,
            AdcWrite::At13Register2 => 2,
            AdcWrite::At13Register4 => 4,
        }
    }

    pub const fn value(self) -> u8 {
        match self {
            AdcWrite::At13Register0 => 0x0F,
            AdcWrite::At13Register1 => 0x01,
            AdcWrite::At13Register2 => 0x7B,
            AdcWrite::At13Register4 => 0x00,
            AdcWrite::At4FRegister1 => 0xA7,
        }
    }
}
