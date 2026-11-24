pub const DAC_FLEXCOMM: u8 = 3;
pub const DAC_ADDRESS: u8 = 0x49;

const MOST_TRANSACTIONS_PER_WRITE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ConfigRegister {
    Register1 = 1,
    Register2 = 2,
    Register7 = 7,
    Register8 = 8,
    Register10 = 10,
    Register12 = 12,
    Register13 = 13,
    Register38 = 38,
    Register39 = 39,
    Register40 = 40,
    Register41 = 41,
    Register46 = 46,
    Register47 = 47,
    Register48 = 48,
    Register49 = 49,
    Register50 = 50,
    Register51 = 51,
    Register52 = 52,
    Register63 = 63,
}

impl ConfigRegister {
    pub const fn factory_value(self) -> u8 {
        match self {
            ConfigRegister::Register1 => 0x00,
            ConfigRegister::Register2 => 0x1C,
            ConfigRegister::Register7 => 0x40,
            ConfigRegister::Register8 => 0x8D,
            ConfigRegister::Register10 => 0x00,
            ConfigRegister::Register12 => 0x5A,
            ConfigRegister::Register13 => 0x20,
            ConfigRegister::Register38 => 0x50,
            ConfigRegister::Register39 => 0x41,
            ConfigRegister::Register40 => 0x72,
            ConfigRegister::Register41 => 0x63,
            ConfigRegister::Register46 => 0x62,
            ConfigRegister::Register47 => 0x3E,
            ConfigRegister::Register48 => 0x00,
            ConfigRegister::Register49 => 0x40,
            ConfigRegister::Register50 => 0x00,
            ConfigRegister::Register51 => 0x40,
            ConfigRegister::Register52 => 0x00,
            ConfigRegister::Register63 => 0x4A,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VolumeRegister {
    FactoryZeroA = 16,
    FactoryZeroB = 17,
    MainA = 18,
    MainB = 19,
    HeadphoneA = 20,
    HeadphoneB = 21,
    FactoryHeldA = 22,
    FactoryHeldB = 23,
}

impl VolumeRegister {
    pub const ALL: [VolumeRegister; 8] = [
        VolumeRegister::FactoryZeroA,
        VolumeRegister::FactoryZeroB,
        VolumeRegister::MainA,
        VolumeRegister::MainB,
        VolumeRegister::HeadphoneA,
        VolumeRegister::HeadphoneB,
        VolumeRegister::FactoryHeldA,
        VolumeRegister::FactoryHeldB,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MasterTrimByte {
    Lowest = 24,
    Low = 25,
    High = 26,
    Highest = 27,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StatusRegister {
    Register113 = 113,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DacRegister {
    Config(ConfigRegister),
    Volume(VolumeRegister),
    MasterTrim(MasterTrimByte),
    Status(StatusRegister),
}

impl DacRegister {
    pub const ALL: [DacRegister; 32] = [
        DacRegister::Config(ConfigRegister::Register1),
        DacRegister::Config(ConfigRegister::Register2),
        DacRegister::Config(ConfigRegister::Register7),
        DacRegister::Config(ConfigRegister::Register8),
        DacRegister::Config(ConfigRegister::Register10),
        DacRegister::Config(ConfigRegister::Register12),
        DacRegister::Config(ConfigRegister::Register13),
        DacRegister::Config(ConfigRegister::Register38),
        DacRegister::Config(ConfigRegister::Register39),
        DacRegister::Config(ConfigRegister::Register40),
        DacRegister::Config(ConfigRegister::Register41),
        DacRegister::Config(ConfigRegister::Register46),
        DacRegister::Config(ConfigRegister::Register47),
        DacRegister::Config(ConfigRegister::Register48),
        DacRegister::Config(ConfigRegister::Register49),
        DacRegister::Config(ConfigRegister::Register50),
        DacRegister::Config(ConfigRegister::Register51),
        DacRegister::Config(ConfigRegister::Register52),
        DacRegister::Config(ConfigRegister::Register63),
        DacRegister::Volume(VolumeRegister::FactoryZeroA),
        DacRegister::Volume(VolumeRegister::FactoryZeroB),
        DacRegister::Volume(VolumeRegister::MainA),
        DacRegister::Volume(VolumeRegister::MainB),
        DacRegister::Volume(VolumeRegister::HeadphoneA),
        DacRegister::Volume(VolumeRegister::HeadphoneB),
        DacRegister::Volume(VolumeRegister::FactoryHeldA),
        DacRegister::Volume(VolumeRegister::FactoryHeldB),
        DacRegister::MasterTrim(MasterTrimByte::Lowest),
        DacRegister::MasterTrim(MasterTrimByte::Low),
        DacRegister::MasterTrim(MasterTrimByte::High),
        DacRegister::MasterTrim(MasterTrimByte::Highest),
        DacRegister::Status(StatusRegister::Register113),
    ];

    pub const fn number(self) -> u8 {
        match self {
            DacRegister::Config(register) => register as u8,
            DacRegister::Volume(register) => register as u8,
            DacRegister::MasterTrim(register) => register as u8,
            DacRegister::Status(register) => register as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attenuation(u8);

impl Attenuation {
    pub const ZERO_DB: Self = Self(0x00);
    pub const FULL: Self = Self(0xFF);

    pub const fn from_half_decibel_steps(steps: u8) -> Self {
        Self(steps)
    }

    pub const fn half_decibel_steps(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MasterTrim(u32);

impl MasterTrim {
    pub const MINUS_ONE_DB: Self = Self(0x7214_82BF);

    pub const fn new(raw: u32) -> Option<Self> {
        if raw <= Self::MINUS_ONE_DB.0 {
            Some(Self(raw))
        } else {
            None
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DacWrite {
    Config(ConfigRegister),
    Volume(VolumeRegister, Attenuation),
    MasterTrim(MasterTrim),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DacTransaction {
    pub register: DacRegister,
    pub value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DacTransactions {
    items: [DacTransaction; MOST_TRANSACTIONS_PER_WRITE],
    len: usize,
}

impl DacTransactions {
    pub fn as_slice(&self) -> &[DacTransaction] {
        &self.items[..self.len]
    }

    const fn one(register: DacRegister, value: u8) -> Self {
        let transaction = DacTransaction { register, value };
        Self {
            items: [transaction; MOST_TRANSACTIONS_PER_WRITE],
            len: 1,
        }
    }
}

impl DacWrite {
    pub const fn transactions(self) -> DacTransactions {
        match self {
            DacWrite::Config(register) => {
                DacTransactions::one(DacRegister::Config(register), register.factory_value())
            }
            DacWrite::Volume(register, level) => {
                DacTransactions::one(DacRegister::Volume(register), level.0)
            }
            DacWrite::MasterTrim(trim) => {
                let [lowest, low, high, highest] = trim.0.to_le_bytes();
                DacTransactions {
                    items: [
                        DacTransaction {
                            register: DacRegister::MasterTrim(MasterTrimByte::Lowest),
                            value: lowest,
                        },
                        DacTransaction {
                            register: DacRegister::MasterTrim(MasterTrimByte::Low),
                            value: low,
                        },
                        DacTransaction {
                            register: DacRegister::MasterTrim(MasterTrimByte::High),
                            value: high,
                        },
                        DacTransaction {
                            register: DacRegister::MasterTrim(MasterTrimByte::Highest),
                            value: highest,
                        },
                    ],
                    len: MOST_TRANSACTIONS_PER_WRITE,
                }
            }
        }
    }
}

#[cfg(doctest)]
#[doc = include_str!("../compile_fail/dac_register_zero_and_sixty_two.md")]
pub struct RegisterZeroAndSixtyTwoAreUnrepresentable;
