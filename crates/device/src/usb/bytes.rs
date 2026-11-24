#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum DescriptorType {
    Device = 0x01,
    Configuration = 0x02,
    String = 0x03,
    Interface = 0x04,
    Endpoint = 0x05,
    DeviceQualifier = 0x06,
    OtherSpeedConfiguration = 0x07,
    InterfaceAssociation = 0x0B,
    ClassInterface = 0x24,
    ClassEndpoint = 0x25,
}

pub(crate) struct DescriptorBytes<const LENGTH: usize> {
    bytes: [u8; LENGTH],
    filled: usize,
}

impl<const LENGTH: usize> DescriptorBytes<LENGTH> {
    pub(crate) const fn empty() -> Self {
        Self {
            bytes: [0; LENGTH],
            filled: 0,
        }
    }

    pub(crate) const fn descriptor(descriptor_type: DescriptorType) -> Self {
        assert!(LENGTH <= u8::MAX as usize, "a descriptor fits bLength");
        Self::empty().byte(LENGTH as u8).byte(descriptor_type as u8)
    }

    pub(crate) const fn byte(mut self, value: u8) -> Self {
        self.bytes[self.filled] = value;
        self.filled += 1;
        self
    }

    pub(crate) const fn word(self, value: u16) -> Self {
        self.append(&value.to_le_bytes())
    }

    pub(crate) const fn double_word(self, value: u32) -> Self {
        self.append(&value.to_le_bytes())
    }

    pub(crate) const fn append(mut self, part: &[u8]) -> Self {
        let mut at = 0;
        while at < part.len() {
            self = self.byte(part[at]);
            at += 1;
        }
        self
    }

    pub(crate) const fn pad_to_end(mut self) -> [u8; LENGTH] {
        self.filled = LENGTH;
        self.finish()
    }

    pub(crate) const fn finish(self) -> [u8; LENGTH] {
        assert!(self.filled == LENGTH, "every declared byte is written");
        self.bytes
    }
}
