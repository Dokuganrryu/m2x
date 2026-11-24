use std::ops::RangeInclusive;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForbiddenBlock {
    FlashController,
    BootRequestRegister,
    DebugMailbox,
    ResetRequestRegister,
}

pub const FLASH_CONTROLLER: RangeInclusive<u32> = 0x4003_4000..=0x4003_4FFF;
pub const FLASH_CONTROLLER_SECURE_ALIAS: RangeInclusive<u32> = 0x5003_4000..=0x5003_4FFF;
pub const AOREG1: RangeInclusive<u32> = 0x4002_0084..=0x4002_0087;
pub const AOREG1_SECURE_ALIAS: RangeInclusive<u32> = 0x5002_0084..=0x5002_0087;
pub const DEBUG_MAILBOX: RangeInclusive<u32> = 0x4009_C000..=0x4009_C0FF;
pub const DEBUG_MAILBOX_SECURE_ALIAS: RangeInclusive<u32> = 0x5009_C000..=0x5009_C0FF;
pub const SCB_AIRCR: RangeInclusive<u32> = 0xE000_ED0C..=0xE000_ED0F;
pub const FACTORY_PAGE: RangeInclusive<u32> = 0x0003_8000..=0x0003_8FFF;
pub const FACTORY_MAGIC: u32 = 0x0003_8200;
pub const FACTORY_SERIAL: u32 = 0x0003_8204;

const FORBIDDEN_EVEN_BY_HIGH_HALF: [(RangeInclusive<u32>, ForbiddenBlock); 6] = [
    (FLASH_CONTROLLER, ForbiddenBlock::FlashController),
    (
        FLASH_CONTROLLER_SECURE_ALIAS,
        ForbiddenBlock::FlashController,
    ),
    (AOREG1, ForbiddenBlock::BootRequestRegister),
    (AOREG1_SECURE_ALIAS, ForbiddenBlock::BootRequestRegister),
    (DEBUG_MAILBOX, ForbiddenBlock::DebugMailbox),
    (DEBUG_MAILBOX_SECURE_ALIAS, ForbiddenBlock::DebugMailbox),
];

const FORBIDDEN_BY_WHOLE_VALUE_ONLY: [(RangeInclusive<u32>, ForbiddenBlock); 1] =
    [(SCB_AIRCR, ForbiddenBlock::ResetRequestRegister)];

pub fn forbidden_block(value: u32) -> Option<ForbiddenBlock> {
    FORBIDDEN_EVEN_BY_HIGH_HALF
        .iter()
        .chain(&FORBIDDEN_BY_WHOLE_VALUE_ONLY)
        .find(|(range, _)| range.contains(&value))
        .map(|(_, block)| *block)
}

pub fn is_factory_page(value: u32) -> bool {
    FACTORY_PAGE.contains(&value)
}

pub fn is_factory_read(value: u32) -> bool {
    value == FACTORY_MAGIC || value == FACTORY_SERIAL
}

pub fn high_half_may_reach_a_guarded_range(high: u16) -> bool {
    FORBIDDEN_EVEN_BY_HIGH_HALF
        .iter()
        .map(|(range, _)| *range.start())
        .chain([*FACTORY_PAGE.start()])
        .any(|start| start >> 16 == u32::from(high))
}
