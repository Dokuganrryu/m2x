use crate::phantom::Milliseconds;

pub const PANEL_COLUMNS: u8 = 128;
pub const PANEL_ROWS: u8 = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hertz(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Microseconds(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpiByteCount(pub u32);

pub const INFERRED_PANEL_SCK: Hertz = Hertz(12_000_000);

const BITS_PER_SPI_BYTE: u64 = 8;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

impl SpiByteCount {
    pub const fn transfer_time(self, clock: Hertz) -> Microseconds {
        let bit_microseconds = self.0 as u64 * BITS_PER_SPI_BYTE * MICROSECONDS_PER_SECOND;
        Microseconds(bit_microseconds.div_ceil(clock.0 as u64) as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpiByte {
    Command(u8),
    Data(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    SoftwareReset,
    SleepOut,
    InferredFrameRateNormal,
    InferredFrameRateIdle,
    InferredFrameRatePartial,
    InferredInversionControl,
    InferredPowerControl1,
    InferredPowerControl2,
    InferredPowerControl3,
    InferredPowerControl4,
    InferredPowerControl5,
    InferredVcomControl,
    MemoryAccessControl,
    InferredGammaPositive,
    InferredGammaNegative,
    UnnamedVendorFc,
    PixelFormat,
    TearingEffectOn,
    ColumnAddressSet,
    RowAddressSet,
    MemoryWrite,
    DisplayOn,
}

impl Opcode {
    pub const fn byte(self) -> u8 {
        match self {
            Opcode::SoftwareReset => 0x01,
            Opcode::SleepOut => 0x11,
            Opcode::InferredFrameRateNormal => 0xB1,
            Opcode::InferredFrameRateIdle => 0xB2,
            Opcode::InferredFrameRatePartial => 0xB3,
            Opcode::InferredInversionControl => 0xB4,
            Opcode::InferredPowerControl1 => 0xC0,
            Opcode::InferredPowerControl2 => 0xC1,
            Opcode::InferredPowerControl3 => 0xC2,
            Opcode::InferredPowerControl4 => 0xC3,
            Opcode::InferredPowerControl5 => 0xC4,
            Opcode::InferredVcomControl => 0xC5,
            Opcode::MemoryAccessControl => 0x36,
            Opcode::InferredGammaPositive => 0xE0,
            Opcode::InferredGammaNegative => 0xE1,
            Opcode::UnnamedVendorFc => 0xFC,
            Opcode::PixelFormat => 0x3A,
            Opcode::TearingEffectOn => 0x35,
            Opcode::ColumnAddressSet => 0x2A,
            Opcode::RowAddressSet => 0x2B,
            Opcode::MemoryWrite => 0x2C,
            Opcode::DisplayOn => 0x29,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelCommand {
    pub opcode: Opcode,
    pub parameters: &'static [u8],
}

impl PanelCommand {
    const fn bare(opcode: Opcode) -> Self {
        Self {
            opcode,
            parameters: &[],
        }
    }

    const fn with(opcode: Opcode, parameters: &'static [u8]) -> Self {
        Self { opcode, parameters }
    }

    pub fn spi_bytes(self) -> impl Iterator<Item = SpiByte> {
        core::iter::once(SpiByte::Command(self.opcode.byte()))
            .chain(self.parameters.iter().map(|value| SpiByte::Data(*value)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitAction {
    ConfigureSpi,
    DriveP1_6High,
    Send(PanelCommand),
    BacklightOn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitStep {
    Act(InitAction),
    Hold(Milliseconds),
    ClearScreen,
}

const FACTORY_RESET_SETTLE: Milliseconds = Milliseconds(150);
const FACTORY_SLEEP_OUT_SETTLE: Milliseconds = Milliseconds(150);

const fn send(command: PanelCommand) -> InitStep {
    InitStep::Act(InitAction::Send(command))
}

pub const INIT_STEPS: [InitStep; 28] = [
    InitStep::Act(InitAction::ConfigureSpi),
    InitStep::Act(InitAction::DriveP1_6High),
    send(PanelCommand::bare(Opcode::SoftwareReset)),
    InitStep::Hold(FACTORY_RESET_SETTLE),
    send(PanelCommand::bare(Opcode::SleepOut)),
    InitStep::Hold(FACTORY_SLEEP_OUT_SETTLE),
    send(PanelCommand::with(
        Opcode::InferredFrameRateNormal,
        &[0x09, 0x3E, 0x3E],
    )),
    send(PanelCommand::with(
        Opcode::InferredFrameRateIdle,
        &[0x05, 0x3A, 0x3A],
    )),
    send(PanelCommand::with(
        Opcode::InferredFrameRatePartial,
        &[0x05, 0x3A, 0x3A, 0x05, 0x3A, 0x3A],
    )),
    send(PanelCommand::with(
        Opcode::InferredInversionControl,
        &[0x03],
    )),
    send(PanelCommand::with(
        Opcode::InferredPowerControl1,
        &[0x28, 0x08, 0x84],
    )),
    send(PanelCommand::with(Opcode::InferredPowerControl2, &[0xC0])),
    send(PanelCommand::with(
        Opcode::InferredPowerControl3,
        &[0x0D, 0x00],
    )),
    send(PanelCommand::with(
        Opcode::InferredPowerControl4,
        &[0x8D, 0x2A],
    )),
    send(PanelCommand::with(
        Opcode::InferredPowerControl5,
        &[0x8D, 0xEE],
    )),
    send(PanelCommand::with(Opcode::InferredVcomControl, &[0x0C])),
    send(PanelCommand::with(Opcode::MemoryAccessControl, &[0x00])),
    send(PanelCommand::with(
        Opcode::InferredGammaPositive,
        &[
            0x05, 0x1A, 0x0C, 0x0E, 0x3A, 0x34, 0x2D, 0x2F, 0x2D, 0x2A, 0x2F, 0x3C, 0x00, 0x01,
            0x02, 0x10,
        ],
    )),
    send(PanelCommand::with(
        Opcode::InferredGammaNegative,
        &[
            0x04, 0x1B, 0x0D, 0x0E, 0x2D, 0x29, 0x24, 0x29, 0x28, 0x26, 0x31, 0x3B, 0x00, 0x00,
            0x03, 0x12,
        ],
    )),
    send(PanelCommand::with(Opcode::UnnamedVendorFc, &[0x80])),
    send(PanelCommand::with(Opcode::PixelFormat, &[0x05])),
    send(PanelCommand::with(Opcode::TearingEffectOn, &[0x00])),
    send(PanelCommand::with(
        Opcode::ColumnAddressSet,
        &[0x00, 0x00, 0x00, PANEL_COLUMNS - 1],
    )),
    send(PanelCommand::with(
        Opcode::RowAddressSet,
        &[0x00, 0x00, 0x00, PANEL_ROWS - 1],
    )),
    send(PanelCommand::bare(Opcode::MemoryWrite)),
    InitStep::ClearScreen,
    send(PanelCommand::bare(Opcode::DisplayOn)),
    InitStep::Act(InitAction::BacklightOn),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RamWindow {
    first_column: u8,
    columns: u8,
    first_row: u8,
    rows: u8,
}

pub const WINDOW_HEADER_SPI_BYTES: SpiByteCount = SpiByteCount(11);
const SPI_BYTES_PER_PIXEL: u32 = 2;

impl RamWindow {
    pub const FULL: Self = Self {
        first_column: 0,
        columns: PANEL_COLUMNS,
        first_row: 0,
        rows: PANEL_ROWS,
    };

    pub(crate) const fn new(first_column: u8, columns: u8, first_row: u8, rows: u8) -> Self {
        assert!(columns > 0 && rows > 0);
        assert!(first_column as u16 + columns as u16 <= PANEL_COLUMNS as u16);
        assert!(first_row as u16 + rows as u16 <= PANEL_ROWS as u16);
        Self {
            first_column,
            columns,
            first_row,
            rows,
        }
    }

    #[cfg(feature = "screen2")]
    pub(crate) const fn checked(
        first_column: u8,
        columns: u8,
        first_row: u8,
        rows: u8,
    ) -> Option<Self> {
        if columns == 0
            || rows == 0
            || first_column as u16 + columns as u16 > PANEL_COLUMNS as u16
            || first_row as u16 + rows as u16 > PANEL_ROWS as u16
        {
            return None;
        }
        Some(Self {
            first_column,
            columns,
            first_row,
            rows,
        })
    }

    pub const fn first_column(self) -> u8 {
        self.first_column
    }

    pub const fn first_row(self) -> u8 {
        self.first_row
    }

    pub const fn columns(self) -> u8 {
        self.columns
    }

    pub const fn rows(self) -> u8 {
        self.rows
    }

    pub const fn pixel_count(self) -> usize {
        self.columns as usize * self.rows as usize
    }

    pub const fn header(self) -> [SpiByte; 11] {
        let last_column = self.first_column + (self.columns - 1);
        let last_row = self.first_row + (self.rows - 1);
        [
            SpiByte::Command(Opcode::ColumnAddressSet.byte()),
            SpiByte::Data(0),
            SpiByte::Data(self.first_column),
            SpiByte::Data(0),
            SpiByte::Data(last_column),
            SpiByte::Command(Opcode::RowAddressSet.byte()),
            SpiByte::Data(0),
            SpiByte::Data(self.first_row),
            SpiByte::Data(0),
            SpiByte::Data(last_row),
            SpiByte::Command(Opcode::MemoryWrite.byte()),
        ]
    }

    pub const fn transfer_bytes(self) -> SpiByteCount {
        SpiByteCount(WINDOW_HEADER_SPI_BYTES.0 + self.pixel_count() as u32 * SPI_BYTES_PER_PIXEL)
    }
}
