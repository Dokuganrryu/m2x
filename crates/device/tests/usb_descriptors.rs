mod descriptor_parser;

use descriptor_parser::{Kind, assert_matches_note, parse, table_after};
use device::usb::{
    BENCH_DEVICE_DESCRIPTOR, DeviceRelease, IdentityRefusal,
    PID_CODES_SHARED_TEST_PAIR_FOR_PRIVATE_BENCH_ONLY_NEVER_SHIPPED, UsbIds, device_descriptor,
};

#[test]
fn the_bench_device_descriptor_carries_every_value_of_the_feedback_note_device_table() {
    let parsed = parse(&BENCH_DEVICE_DESCRIPTOR);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].kind, Kind::Device);
    assert_matches_note(&parsed[0], &table_after("### Device descriptor"), 1);
}

#[test]
fn only_the_open_pid_codes_vendor_is_accepted() {
    for vendor in [0x0000, 0x0483, 0x1234, 0xFFFF] {
        assert_eq!(
            UsbIds::new(vendor, 0x0001),
            Err(IdentityRefusal::NotTheOpenPidCodesVendor)
        );
    }
    assert!(UsbIds::new(0x1209, 0x0001).is_ok());
}

#[test]
fn the_bench_default_is_the_pid_codes_test_pair_1209_0001() {
    let bench = PID_CODES_SHARED_TEST_PAIR_FOR_PRIVATE_BENCH_ONLY_NEVER_SHIPPED;
    assert_eq!(
        (bench.vendor().value(), bench.product().value()),
        (0x1209, 0x0001)
    );
    let parsed = parse(&BENCH_DEVICE_DESCRIPTOR);
    assert_eq!(parsed[0].field("idVendor"), 0x1209);
    assert_eq!(parsed[0].field("idProduct"), 0x0001);
}

#[test]
fn vendor_product_and_release_are_parameters_of_the_device_descriptor() {
    let ids = UsbIds::new(0x1209, 0xABCD).expect("a pid.codes pair");
    let release = DeviceRelease::new(2, 17).expect("a two-digit release");
    let parsed = parse(&device_descriptor(ids, release));
    assert_eq!(parsed[0].field("idVendor"), 0x1209);
    assert_eq!(parsed[0].field("idProduct"), 0xABCD);
    assert_eq!(parsed[0].field("bcdDevice"), 0x0217);
}

#[test]
fn a_release_number_past_two_decimal_digits_is_refused() {
    assert_eq!(DeviceRelease::new(100, 0), None);
    assert_eq!(DeviceRelease::new(1, 100), None);
}

#[test]
fn the_device_descriptor_names_manufacturer_product_and_serial_strings_one_two_and_three() {
    let parsed = parse(&BENCH_DEVICE_DESCRIPTOR);
    assert_eq!(
        [
            parsed[0].field("iManufacturer"),
            parsed[0].field("iProduct"),
            parsed[0].field("iSerialNumber")
        ],
        [1, 2, 3]
    );
}
