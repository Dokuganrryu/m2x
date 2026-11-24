#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VendorId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbIds {
    vendor: VendorId,
    product: ProductId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityRefusal {
    NotTheOpenPidCodesVendor,
}

// WHY: The image may only enumerate under the open pid.codes vendor ID, so it can never claim a commercial vendor's ID and bind that vendor's driver
pub const OPEN_VENDOR_ID: VendorId = VendorId(0x1209);

pub const PID_CODES_SHARED_TEST_PAIR_FOR_PRIVATE_BENCH_ONLY_NEVER_SHIPPED: UsbIds = UsbIds {
    vendor: VendorId(0x1209),
    product: ProductId(0x0001),
};

impl VendorId {
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl ProductId {
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl UsbIds {
    pub const fn new(vendor: u16, product: u16) -> Result<Self, IdentityRefusal> {
        if vendor != OPEN_VENDOR_ID.0 {
            return Err(IdentityRefusal::NotTheOpenPidCodesVendor);
        }
        Ok(Self {
            vendor: VendorId(vendor),
            product: ProductId(product),
        })
    }

    pub const fn vendor(self) -> VendorId {
        self.vendor
    }

    pub const fn product(self) -> ProductId {
        self.product
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceRelease {
    binary_coded_decimal: u16,
}

impl DeviceRelease {
    pub const FIRST: Self = Self {
        binary_coded_decimal: 0x0100,
    };

    pub const fn new(major: u8, minor: u8) -> Option<Self> {
        if major > 99 || minor > 99 {
            return None;
        }
        let digits = [major / 10, major % 10, minor / 10, minor % 10];
        Some(Self {
            binary_coded_decimal: (digits[0] as u16) << 12
                | (digits[1] as u16) << 8
                | (digits[2] as u16) << 4
                | digits[3] as u16,
        })
    }

    pub const fn binary_coded_decimal(self) -> u16 {
        self.binary_coded_decimal
    }
}
