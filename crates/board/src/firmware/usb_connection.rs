use device::usb_detach::{self, UsbDetach};

use super::map::{
    SYSCON_AHBCLKCTRL2, SYSCON_AHBCLKCTRL2_USB1_DEV, USBHSD_DEVCMDSTAT, USBHSD_DEVCMDSTAT_DCON,
    USBHSD_DEVCMDSTAT_FORCE_NEEDCLK, USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR,
};

pub fn detach() {
    let usb_device_clock_is_on = SYSCON_AHBCLKCTRL2.read() & SYSCON_AHBCLKCTRL2_USB1_DEV != 0;
    match usb_detach::detach_decision(usb_device_clock_is_on) {
        UsbDetach::LeaveTheControllerAlone => {}
        UsbDetach::DetachTheController => detach_connected_device(),
    }
}

fn detach_connected_device() {
    USBHSD_DEVCMDSTAT.write(
        (USBHSD_DEVCMDSTAT.read()
            & !(USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR | USBHSD_DEVCMDSTAT_DCON))
            | USBHSD_DEVCMDSTAT_FORCE_NEEDCLK,
    );
}
