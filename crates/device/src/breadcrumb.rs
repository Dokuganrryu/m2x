use crate::fault::FaultCode;

pub const MAGIC: u32 = 0x4D32_4243;

const STEP_SHIFT: u32 = 8;
const CODE_SHIFT: u32 = 16;
const LATEST_SHIFT: u32 = 24;
const BYTE: u32 = 0xFF;
const CODE_MASK: u32 = 0x7F;
const RAISED: u32 = 1 << 23;

const _: () = {
    let mut index = 0;
    while index < FaultCode::ALL.len() {
        assert!(
            FaultCode::ALL[index].number() as u32 <= CODE_MASK,
            "a fault code leaves bit 7 of its byte to the weight"
        );
        index += 1;
    }
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub stage: u8,
    pub step: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Noted,
    Raised,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    Here,
    Step(Place),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Breadcrumb {
    pub place: Place,
    pub code: u8,
    pub weight: Weight,
    pub latest: u8,
}

impl Breadcrumb {
    pub const START: Self = Self {
        place: Place { stage: 1, step: 0 },
        code: 0,
        weight: Weight::Noted,
        latest: 1,
    };

    pub const fn previous(words: [u32; 2]) -> Option<Self> {
        if words[0] == MAGIC {
            Some(Self::unpack(words[1]))
        } else {
            None
        }
    }

    pub const fn pack(self) -> u32 {
        let weight = match self.weight {
            Weight::Noted => 0,
            Weight::Raised => RAISED,
        };
        self.place.stage as u32
            | (self.place.step as u32) << STEP_SHIFT
            | (self.code as u32 & CODE_MASK) << CODE_SHIFT
            | weight
            | (self.latest as u32) << LATEST_SHIFT
    }

    pub const fn unpack(word: u32) -> Self {
        Self {
            place: Place {
                stage: (word & BYTE) as u8,
                step: (word >> STEP_SHIFT & BYTE) as u8,
            },
            code: (word >> CODE_SHIFT & CODE_MASK) as u8,
            weight: if word & RAISED == 0 {
                Weight::Noted
            } else {
                Weight::Raised
            },
            latest: (word >> LATEST_SHIFT) as u8,
        }
    }

    pub const fn marked(self, place: Place) -> Self {
        Self {
            place: if self.code == 0 { place } else { self.place },
            latest: place.stage,
            ..self
        }
    }

    pub const fn recorded(self, code: FaultCode, at: At, weight: Weight) -> Self {
        let empty = self.code == 0;
        let outweighed = matches!((self.weight, weight), (Weight::Noted, Weight::Raised));
        if !(empty || outweighed) {
            return self;
        }
        let place = match at {
            At::Step(place) => place,
            At::Here if empty => self.place,
            At::Here => Place {
                stage: self.latest,
                step: 0,
            },
        };
        Self {
            place,
            code: code.number(),
            weight,
            latest: self.latest,
        }
    }
}
