use crate::lcd::panel::{Opcode, PanelCommand, RamWindow};
use crate::lcd::screen::Rgb565;

const CONTROL_SHIFT: u32 = 16;
// WHY: FIFOWR control halves 0x074C (8-bit, D/C low, no EOT) and 0x0F4E (16-bit, D/C high, no EOT)
const EIGHT_BIT_COMMAND_FRAME: u32 = 0x074C << CONTROL_SHIFT;
const SIXTEEN_BIT_DATA_FRAME: u32 = 0x0F4E << CONTROL_SHIFT;
// WHY: D/C is FIFOWR bit 17, and 0x075C and 0x0F5E differ from their no-EOT halves only in EOT
const DATA_NOT_COMMAND: u32 = 1 << 17;
const END_OF_TRANSFER: u32 = 0x0010 << CONTROL_SHIFT;
const LOW_HALF: u32 = 0xFFFF;
const GREEN_FIELD: u16 = 0x07E0;
const FIVE_BIT_FIELD: u16 = 0x1F;
const RED_SHIFT: u16 = 11;

// WHY: The panel reads each pixel's fields as blue, green, red under the factory MADCTL 0x00, so the init keeps MADCTL 0x00 and each pixel leaves with red and blue swapped
const fn panel_order(bits: u16) -> u16 {
    (bits & GREEN_FIELD) | (bits >> RED_SHIFT) | ((bits & FIVE_BIT_FIELD) << RED_SHIFT)
}

pub const WINDOW_HEADER_WORDS: usize = 7;
pub const MOST_COMMAND_WORDS: usize = 17;
// WHY: UM11126 Table 438, XFERCOUNT is 10 bits and reads 0x3FF once the descriptor is exhausted, so a 1024-transfer descriptor would read as done before it starts
pub const MOST_TRANSFERS_PER_DESCRIPTOR: usize = 1023;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelWord(u32);

impl PanelWord {
    pub const UNSENT: Self = Self(0);

    pub const fn command(opcode: Opcode) -> Self {
        Self(EIGHT_BIT_COMMAND_FRAME | opcode.byte() as u32)
    }

    pub const fn parameter(byte: u8) -> Self {
        Self(EIGHT_BIT_COMMAND_FRAME | DATA_NOT_COMMAND | byte as u32)
    }

    pub const fn halfword(value: u16) -> Self {
        Self(SIXTEEN_BIT_DATA_FRAME | value as u32)
    }

    pub const fn pixel(colour: Rgb565) -> Self {
        Self::halfword(panel_order(colour.bits()))
    }

    pub const fn ending(self) -> Self {
        Self(self.0 | END_OF_TRANSFER)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn colour(self) -> Rgb565 {
        Rgb565::from_bits(panel_order((self.0 & LOW_HALF) as u16))
    }
}

// WHY: Each task goes out with one D/C level and ends with EOT, the opcode as a task of its own, so D/C (SSEL1) never changes inside a transfer; UM11126 35.7.5 does not say a TXSSEL change takes effect without an EOT
pub fn command_words(
    command: PanelCommand,
    store: &mut [PanelWord; MOST_COMMAND_WORDS],
) -> &[PanelWord] {
    store[0] = PanelWord::command(command.opcode).ending();
    let mut len = 1;
    for (slot, byte) in store[1..].iter_mut().zip(command.parameters) {
        *slot = PanelWord::parameter(*byte);
        len += 1;
    }
    let words = &mut store[..len];
    if let Some(last) = words.last_mut() {
        *last = last.ending();
    }
    words
}

pub const fn window_header(window: RamWindow) -> [PanelWord; WINDOW_HEADER_WORDS] {
    let last_column = window.first_column() + (window.columns() - 1);
    let last_row = window.first_row() + (window.rows() - 1);
    [
        PanelWord::command(Opcode::ColumnAddressSet).ending(),
        PanelWord::halfword(window.first_column() as u16),
        PanelWord::halfword(last_column as u16).ending(),
        PanelWord::command(Opcode::RowAddressSet).ending(),
        PanelWord::halfword(window.first_row() as u16),
        PanelWord::halfword(last_row as u16).ending(),
        PanelWord::command(Opcode::MemoryWrite).ending(),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub first: usize,
    pub count: usize,
}

pub fn descriptor_chunks(words: usize) -> impl Iterator<Item = Chunk> {
    (0..words.div_ceil(MOST_TRANSFERS_PER_DESCRIPTOR)).map(move |index| {
        let first = index * MOST_TRANSFERS_PER_DESCRIPTOR;
        Chunk {
            first,
            count: (words - first).min(MOST_TRANSFERS_PER_DESCRIPTOR),
        }
    })
}
