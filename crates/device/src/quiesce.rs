#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockStop {
    LeaveTheBlockAlone,
    StopTheBlock,
}

// WHY: The loader hands over AHBCLKCTRL0 0x000EE1FB and AHBCLKCTRL1 0x00004000, so DMA0, CTIMER0 and FC0, FC2, FC6 and FC7 are unclocked; a block with its bus clock off cannot run, so its stop is a no-op and the access is left out, as usb_detach leaves an unclocked USB1 alone
pub const fn bus_block_stop(bus_clock_is_on: bool) -> BlockStop {
    if bus_clock_is_on {
        BlockStop::StopTheBlock
    } else {
        BlockStop::LeaveTheBlockAlone
    }
}

// WHY: SVD28 FLEXCOMM PSELID PERSEL, CFG1 at +0xC00 is the I2S register only while PERSEL selects I2S transmit or receive and PSELID resets to 0, no function selected
pub const fn i2s_stop(bus_clock_is_on: bool, selected_as_i2s: bool) -> BlockStop {
    if bus_clock_is_on && selected_as_i2s {
        BlockStop::StopTheBlock
    } else {
        BlockStop::LeaveTheBlockAlone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_the_bootloader_left_clocked_is_stopped() {
        assert_eq!(bus_block_stop(true), BlockStop::StopTheBlock);
    }

    #[test]
    fn a_block_the_bootloader_left_unclocked_gets_no_access_at_all() {
        assert_eq!(bus_block_stop(false), BlockStop::LeaveTheBlockAlone);
    }

    #[test]
    fn a_clocked_flexcomm_selected_as_i2s_is_stopped() {
        assert_eq!(i2s_stop(true, true), BlockStop::StopTheBlock);
    }

    #[test]
    fn a_clocked_flexcomm_with_no_i2s_selected_keeps_its_registers_untouched() {
        assert_eq!(i2s_stop(true, false), BlockStop::LeaveTheBlockAlone);
    }

    #[test]
    fn an_unclocked_flexcomm_gets_no_access_at_all() {
        assert_eq!(i2s_stop(false, false), BlockStop::LeaveTheBlockAlone);
        assert_eq!(i2s_stop(false, true), BlockStop::LeaveTheBlockAlone);
    }
}
