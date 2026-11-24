use super::map::{
    MAIR_DEVICE_NGNRE, MAIR_NORMAL_NON_CACHEABLE, MAIR_NORMAL_WRITE_BACK, MPU_ADDRESS_GRANULE,
    MPU_CTRL, MPU_CTRL_ENABLE, MPU_CTRL_OFF, MPU_MAIR0, MPU_RBAR,
    MPU_RBAR_AP_READ_WRITE_PRIVILEGED, MPU_RBAR_SH_NON_SHAREABLE, MPU_RBAR_XN, MPU_RLAR,
    MPU_RLAR_ATTRINDX_SHIFT, MPU_RLAR_DISABLED, MPU_RLAR_EN, MPU_RNR, MPU_TYPE,
    MPU_TYPE_DREGION_MASK, MPU_TYPE_DREGION_SHIFT, USB1_RAM_FIRST_BYTE,
};
use super::mmio::{
    data_memory_barrier, data_synchronization_barrier, instruction_synchronization_barrier,
};
use device::usb::endpoint::USB_RAM_BYTES;

#[derive(Clone, Copy)]
enum Attributes {
    NormalWriteBack,
    Device,
    NormalNonCacheable,
}

impl Attributes {
    const fn index(self) -> u32 {
        match self {
            Attributes::NormalWriteBack => 0,
            Attributes::Device => 1,
            Attributes::NormalNonCacheable => 2,
        }
    }

    const fn encoding(self) -> u32 {
        match self {
            Attributes::NormalWriteBack => MAIR_NORMAL_WRITE_BACK,
            Attributes::Device => MAIR_DEVICE_NGNRE,
            Attributes::NormalNonCacheable => MAIR_NORMAL_NON_CACHEABLE,
        }
    }

    const fn field(self) -> u32 {
        self.encoding() << (8 * self.index())
    }

    const fn mair0() -> u32 {
        Attributes::NormalWriteBack.field()
            | Attributes::Device.field()
            | Attributes::NormalNonCacheable.field()
    }
}

const fn mair0_carries(attributes: Attributes) -> bool {
    (Attributes::mair0() >> (8 * attributes.index())) & 0xFF == attributes.encoding()
}

const _: () = assert!(
    mair0_carries(Attributes::NormalWriteBack)
        && mair0_carries(Attributes::Device)
        && mair0_carries(Attributes::NormalNonCacheable)
);

// WHY: Attr0 0xFF Normal write-back, Attr1 0x04 Device nGnRE and Attr2 0x44 Normal Non-cacheable; the assertion above builds both sides from encoding(), so only a literal catches a wrong constant
const _: () = assert!(Attributes::mair0() == 0x0044_04FF);

#[derive(Clone, Copy)]
enum Execution {
    Allowed,
    Never,
}

#[derive(Clone, Copy)]
struct Region {
    first_byte: u32,
    last_byte: u32,
    attributes: Attributes,
    execution: Execution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceRefusal {
    TooFewRegions { implemented: u32 },
    RegionOffGranule,
    RegionNotHeld { region: u32 },
}

impl Region {
    const fn on_granule(self) -> bool {
        self.first_byte % MPU_ADDRESS_GRANULE == 0
            && (self.last_byte % MPU_ADDRESS_GRANULE) == MPU_ADDRESS_GRANULE - 1
            && self.first_byte < self.last_byte
    }

    const fn base_word(self) -> u32 {
        let execute_never = match self.execution {
            Execution::Allowed => 0,
            Execution::Never => MPU_RBAR_XN,
        };
        self.first_byte
            | MPU_RBAR_SH_NON_SHAREABLE
            | MPU_RBAR_AP_READ_WRITE_PRIVILEGED
            | execute_never
    }

    const fn limit_word(self) -> u32 {
        (self.last_byte & !(MPU_ADDRESS_GRANULE - 1))
            | (self.attributes.index() << MPU_RLAR_ATTRINDX_SHIFT)
            | MPU_RLAR_EN
    }
}

// WHY: The flash controller block is exactly the 4 KB gap between these two regions, and address bit 28 selects the secure alias, so every secure alias stays fenced
const PERIPHERALS_BELOW_THE_FENCED_BLOCK: Region = Region {
    first_byte: 0x4000_0000,
    last_byte: 0x4003_3FFF,
    attributes: Attributes::Device,
    execution: Execution::Never,
};
const PERIPHERALS_BETWEEN_THE_FENCED_BLOCK_AND_USB_RAM: Region = Region {
    first_byte: 0x4003_5000,
    last_byte: 0x400F_FFFF,
    attributes: Attributes::Device,
    execution: Execution::Never,
};
// WHY: Arm DDI 0553A.h B5.16 RCSKG, any unaligned access to Device memory raises an UNALIGNED UsageFault, and UM11126 rev 1.8 section 44.1 points at the MPU to map USB RAM otherwise; Non-cacheable Normal is shareable whatever SH says and coherent with the controller, B5.14 RLCPJ and B5.15 RFTKW
const USB_RAM: Region = Region {
    first_byte: 0x4010_0000,
    last_byte: 0x4010_3FFF,
    attributes: Attributes::NormalNonCacheable,
    execution: Execution::Never,
};
const PERIPHERALS_ABOVE_USB_RAM: Region = Region {
    first_byte: 0x4010_4000,
    last_byte: 0x4FFF_FFFF,
    attributes: Attributes::Device,
    execution: Execution::Never,
};

const PERIPHERAL_REGIONS: [Region; 4] = [
    PERIPHERALS_BELOW_THE_FENCED_BLOCK,
    PERIPHERALS_BETWEEN_THE_FENCED_BLOCK_AND_USB_RAM,
    USB_RAM,
    PERIPHERALS_ABOVE_USB_RAM,
];

const fn overlap(first: Region, second: Region) -> bool {
    first.first_byte <= second.last_byte && second.first_byte <= first.last_byte
}

const fn covers_usb_ram_as_non_cacheable_normal_memory(region: Region) -> bool {
    region.first_byte == USB1_RAM_FIRST_BYTE
        && region.last_byte == USB1_RAM_FIRST_BYTE + (USB_RAM_BYTES as u32 - 1)
        && matches!(region.attributes, Attributes::NormalNonCacheable)
}

const fn fence_maps_usb_ram_as_normal_memory_without_overlap(regions: &[Region]) -> bool {
    let mut usb_ram_regions = 0;
    let mut at = 0;
    while at < regions.len() {
        if !regions[at].on_granule() {
            return false;
        }
        if covers_usb_ram_as_non_cacheable_normal_memory(regions[at]) {
            usb_ram_regions += 1;
        }
        let mut other = at + 1;
        while other < regions.len() {
            if overlap(regions[at], regions[other]) {
                return false;
            }
            other += 1;
        }
        at += 1;
    }
    usb_ram_regions == 1
}

const _: () = assert!(fence_maps_usb_ram_as_normal_memory_without_overlap(
    &PERIPHERAL_REGIONS
));

const REGIONS_IN_THE_FENCE: u32 = 1 + PERIPHERAL_REGIONS.len() as u32;
// WHY: Arm 100235 MPU_CTRL, PRIVDEFENA 0 faults every access outside an enabled region, HFNMIENA 0 turns the MPU off in HardFault and NMI
const ENABLED_WITHOUT_BACKGROUND_MAP_AND_OFF_IN_HARD_FAULT: u32 = MPU_CTRL_ENABLE;

unsafe extern "C" {
    static __vector_table: u32;
    static _stack_start: u32;
}

fn image_and_stack() -> Region {
    let first_byte = (&raw const __vector_table) as u32;
    let end = (&raw const _stack_start) as u32;
    Region {
        first_byte,
        last_byte: end.wrapping_sub(1),
        attributes: Attributes::NormalWriteBack,
        execution: Execution::Allowed,
    }
}

pub fn raise() -> Result<(), FenceRefusal> {
    let [below, between, usb_ram, above] = PERIPHERAL_REGIONS;
    let regions = [image_and_stack(), below, between, usb_ram, above];
    let implemented = (MPU_TYPE.read() >> MPU_TYPE_DREGION_SHIFT) & MPU_TYPE_DREGION_MASK;
    if implemented < REGIONS_IN_THE_FENCE {
        return Err(FenceRefusal::TooFewRegions { implemented });
    }
    if !regions.iter().all(|region| region.on_granule()) {
        return Err(FenceRefusal::RegionOffGranule);
    }
    data_memory_barrier();
    MPU_CTRL.write(MPU_CTRL_OFF);
    data_synchronization_barrier();
    instruction_synchronization_barrier();
    MPU_MAIR0.write(Attributes::mair0());
    let mut number = 0;
    for region in regions {
        MPU_RNR.write(number);
        MPU_RBAR.write(region.base_word());
        MPU_RLAR.write(region.limit_word());
        if MPU_RBAR.read() != region.base_word() || MPU_RLAR.read() != region.limit_word() {
            return Err(FenceRefusal::RegionNotHeld { region: number });
        }
        number += 1;
    }
    while number < implemented {
        MPU_RNR.write(number);
        MPU_RLAR.write(MPU_RLAR_DISABLED);
        if MPU_RLAR.read() != MPU_RLAR_DISABLED {
            return Err(FenceRefusal::RegionNotHeld { region: number });
        }
        number += 1;
    }
    MPU_CTRL.write(ENABLED_WITHOUT_BACKGROUND_MAP_AND_OFF_IN_HARD_FAULT);
    data_synchronization_barrier();
    instruction_synchronization_barrier();
    Ok(())
}
