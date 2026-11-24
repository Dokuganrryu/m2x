#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbDetach {
    LeaveTheControllerAlone,
    DetachTheController,
}

pub const fn detach_decision(usb_device_clock_is_on: bool) -> UsbDetach {
    if usb_device_clock_is_on {
        UsbDetach::DetachTheController
    } else {
        UsbDetach::LeaveTheControllerAlone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bootloader_that_left_the_usb_device_clock_on_asks_for_a_detach() {
        assert_eq!(detach_decision(true), UsbDetach::DetachTheController);
    }

    #[test]
    fn a_bootloader_that_left_the_usb_device_clock_off_asks_for_no_access_at_all() {
        assert_eq!(detach_decision(false), UsbDetach::LeaveTheControllerAlone);
    }
}
