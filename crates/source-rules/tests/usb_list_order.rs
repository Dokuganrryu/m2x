use std::fs;
use std::path::Path;

fn usb_controller() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("source-rules sits two folders below the root")
        .join("crates/board/src/firmware/usb_controller.rs");
    fs::read_to_string(path).expect("usb_controller.rs is readable")
}

fn body_of<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("usb_controller.rs defines `{signature}`"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`{signature}` ends at a closing brace in column one"));
    &rest[..end]
}

fn at(body: &str, needle: &str, what: &str) -> usize {
    body.rfind(needle)
        .unwrap_or_else(|| panic!("`arm` has {what} (`{needle}`)"))
}

#[test]
fn arm_orders_the_buffer_and_epinuse_writes_before_active() {
    let source = usb_controller();
    let arm = body_of(&source, "fn arm(entry: ListEntry, bytes: ByteCount)");
    let epinuse = at(arm, "point_controller_at(", "the EPINUSE write");
    let active = at(
        arm,
        "list_word(entry.list_offset()).write(armed_word(entry, bytes))",
        "the ACTIVE list-word write",
    );
    let after_epinuse = &arm[epinuse..active];
    let block_end = after_epinuse
        .find("\n    }\n")
        .expect("the block that holds the EPINUSE write closes before the ACTIVE list-word write");
    let barrier = after_epinuse[block_end..]
        .lines()
        .any(|line| line == "    data_memory_barrier();");
    assert!(
        barrier,
        "no data_memory_barrier() between the EPINUSE write and the ACTIVE list-word write in `arm`; B5.10 RVMHG orders one peripheral only, so B5.13.2 RWMRT needs a DMB there"
    );
    let stray = after_epinuse[block_end + "\n    }\n".len()..]
        .lines()
        .find(|line| {
            !(line.trim().is_empty()
                || line.starts_with("    // ")
                || *line == "    data_memory_barrier();")
        });
    assert!(
        stray.is_none(),
        "`{}` sits between the EPINUSE block and the ACTIVE list-word write in `arm`; only the barrier and its line comments belong there, because an attribute or a block comment can remove the DMB",
        stray.unwrap_or_default().trim()
    );
}

fn first(body: &str, needle: &str, what: &str) -> usize {
    body.find(needle)
        .unwrap_or_else(|| panic!("`attach` has {what} (`{needle}`)"))
}

#[test]
fn attach_selects_device_mode_and_brings_up_the_phy_before_any_usb_device_register() {
    let source = usb_controller();
    let attach = body_of(&source, "pub fn attach<Lock>(");
    let device_register = first(attach, "USBHSD_", "a USB1 device register access");
    let port_mode = first(
        attach,
        "USBHSH_PORTMODE.write(",
        "the PORTMODE device-mode write",
    );
    let last_phy = at(attach, "USBPHY_", "a USB PHY register write");
    let lock = first(attach, "phy_pll_lock()", "the PHY PLL lock wait");
    assert!(
        port_mode < device_register,
        "`attach` touches a USB1 device register before USBHSH_PORTMODE selects device mode; the controller only arrives on the bus when PORTMODE is written first"
    );
    assert!(
        lock < device_register && last_phy < device_register,
        "`attach` touches a USB1 device register before the PHY writes and the PLL lock wait end; the PHY must be up before the first USBHSD access or the device never arrives"
    );
}
