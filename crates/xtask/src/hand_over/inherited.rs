#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaChannel(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRange {
    pub channel: DmaChannel,
    pub what: &'static str,
    pub start: u32,
    pub end: u32,
}

impl LiveRange {
    pub const fn holds(&self, address: u32, size: u32) -> bool {
        address < self.end && address + size > self.start
    }

    pub const fn overlaps(&self, start: u32, end: u32) -> bool {
        start < self.end && end > self.start
    }
}

const fn range(channel: u8, what: &'static str, start: u32, bytes: u32) -> LiveRange {
    LiveRange {
        channel: DmaChannel(channel),
        what,
        start,
        end: start + bytes,
    }
}

const DESCRIPTOR_BYTES: u32 = 16;
const AUDIO_BUFFER_BYTES: u32 = 0x300;

// WHY: A synthetic layout for the image that ran before the load: the DMA head table, reload descriptors and audio buffers sit inside this image's .data and .bss, so any write before the quiesce that lands on them is caught
const SRAMBASE: u32 = 0x2002_C000;
const fn head_entry(channel: u32) -> u32 {
    SRAMBASE + channel * DESCRIPTOR_BYTES
}
const FC2_TX_BUFFER: u32 = 0x2002_A000;
const FC0_TX_BUFFER: u32 = 0x2002_A300;
const FC6_RX_BUFFER: u32 = 0x2002_A600;
const FC7_RX_BUFFER: u32 = 0x2002_A900;
const FC7_RX_RELOAD: u32 = 0x2002_C200;
const FC6_RX_RELOAD: u32 = 0x2002_C210;
const FC2_TX_RELOAD: u32 = 0x2002_C220;
const FC0_TX_RELOAD: u32 = 0x2002_C230;
const LCD_CHAIN: u32 = 0x2002_C400;
const LCD_CHAIN_BYTES: u32 = 0x260;
const LCD_STREAM: u32 = 0x2001_0000;
const LCD_STREAM_END: u32 = 0x2002_8000;
const LAST_WORD: u32 = AUDIO_BUFFER_BYTES - 4;

pub const LIVE_RANGES: [LiveRange; 15] = [
    range(5, "FC0 TX buffer", FC0_TX_BUFFER, AUDIO_BUFFER_BYTES),
    range(
        5,
        "FC0 TX reload descriptor",
        FC0_TX_RELOAD,
        DESCRIPTOR_BYTES,
    ),
    range(5, "FC0 TX head entry", head_entry(5), DESCRIPTOR_BYTES),
    range(11, "FC2 TX buffer", FC2_TX_BUFFER, AUDIO_BUFFER_BYTES),
    range(
        11,
        "FC2 TX reload descriptor",
        FC2_TX_RELOAD,
        DESCRIPTOR_BYTES,
    ),
    range(11, "FC2 TX head entry", head_entry(11), DESCRIPTOR_BYTES),
    range(16, "FC6 RX buffer", FC6_RX_BUFFER, AUDIO_BUFFER_BYTES),
    range(
        16,
        "FC6 RX reload descriptor",
        FC6_RX_RELOAD,
        DESCRIPTOR_BYTES,
    ),
    range(16, "FC6 RX head entry", head_entry(16), DESCRIPTOR_BYTES),
    range(18, "FC7 RX buffer", FC7_RX_BUFFER, AUDIO_BUFFER_BYTES),
    range(
        18,
        "FC7 RX reload descriptor",
        FC7_RX_RELOAD,
        DESCRIPTOR_BYTES,
    ),
    range(18, "FC7 RX head entry", head_entry(18), DESCRIPTOR_BYTES),
    range(13, "LCD head entry", head_entry(13), DESCRIPTOR_BYTES),
    range(13, "LCD descriptor chain", LCD_CHAIN, LCD_CHAIN_BYTES),
    range(
        13,
        "LCD framebuffer stream",
        LCD_STREAM,
        LCD_STREAM_END - LCD_STREAM,
    ),
];

pub const TRANSMIT_CHANNELS: [DmaChannel; 2] = [DmaChannel(5), DmaChannel(11)];

pub const fn channel_bits(channels: &[DmaChannel]) -> u32 {
    let mut bits = 0;
    let mut index = 0;
    while index < channels.len() {
        bits |= 1 << channels[index].0;
        index += 1;
    }
    bits
}

// WHY: The four audio channels and the screen's channel 13 run while the previous image plays
pub const INHERITED_ENABLED_CHANNELS: u32 = channel_bits(&[
    DmaChannel(5),
    DmaChannel(11),
    DmaChannel(13),
    DmaChannel(16),
    DmaChannel(18),
]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poke {
    Word(u32, u32),
    Byte(u32, u8),
}

pub struct InheritedItem {
    pub what: &'static str,
    pub pokes: &'static [Poke],
}

const fn descriptor(at: u32, transfer: u32, source_end: u32, destination_end: u32) -> [Poke; 4] {
    [
        Poke::Word(at, transfer),
        Poke::Word(at + 4, source_end),
        Poke::Word(at + 8, destination_end),
        Poke::Word(at + 12, at),
    ]
}

const FC0_TX_DESCRIPTOR: [Poke; 4] = descriptor(
    FC0_TX_RELOAD,
    0x00BF_1207,
    FC0_TX_BUFFER + LAST_WORD,
    0x4008_6E20,
);
const FC2_TX_DESCRIPTOR: [Poke; 4] = descriptor(
    FC2_TX_RELOAD,
    0x00BF_1207,
    FC2_TX_BUFFER + LAST_WORD,
    0x4008_8E20,
);
const FC6_RX_DESCRIPTOR: [Poke; 4] = descriptor(
    FC6_RX_RELOAD,
    0x00BF_4207,
    0x4009_7E30,
    FC6_RX_BUFFER + LAST_WORD,
);
const FC7_RX_DESCRIPTOR: [Poke; 4] = descriptor(
    FC7_RX_RELOAD,
    0x00BF_4207,
    0x4009_8E30,
    FC7_RX_BUFFER + LAST_WORD,
);

const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ISER1: u32 = 0xE000_E104;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ISPR1: u32 = 0xE000_E204;
const NVIC_IPR: u32 = 0xE000_E400;
const CTIMER0_IRQ: u32 = 10;
const USB1_IRQ: u32 = 47;
const FLEXCOMM5_IRQ: u32 = 19;
const FLEXCOMM1_IRQ: u32 = 15;
const PIN_INT0_IRQ: u32 = 4;
const PIN_INT1_IRQ: u32 = 5;
const INHERITED_PRIORITY: u8 = 0x20;
const ICSR: u32 = 0xE000_ED04;
const ICSR_PENDSTSET: u32 = 1 << 26;
const VTOR: u32 = 0xE000_ED08;
const SYST_CSR: u32 = 0xE000_E010;
const SYST_RVR: u32 = 0xE000_E014;
const DMA0_CTRL: u32 = 0x4008_2000;
const DMA0_SRAMBASE: u32 = 0x4008_2008;
const DMA0_ENABLESET0: u32 = 0x4008_2020;
const CTIMER0_TCR: u32 = 0x4000_8004;
const I2S_CFG1: [u32; 4] = [0x4008_6C00, 0x4008_8C00, 0x4009_7C00, 0x4009_8C00];
const I2S_CFG1_MAINENABLE: u32 = 1;
const USBHSD_DEVCMDSTAT: u32 = 0x4009_4000;
const USBHSD_DEVCMDSTAT_DEV_EN_AND_DCON: u32 = (1 << 7) | (1 << 16);
const SYSCON_AHBCLKCTRL0: u32 = 0x4000_0200;
const SYSCON_AHBCLKCTRL0_DMA0: u32 = 1 << 20;
const SYSCON_AHBCLKCTRL1: u32 = 0x4000_0204;
const SYSCON_AHBCLKCTRL1_TIMER0_AND_I2S_FLEXCOMMS: u32 =
    (1 << 26) | (1 << 18) | (1 << 17) | (1 << 13) | (1 << 11);
const FLEXCOMM_PSELID: [u32; 4] = [0x4008_6FF8, 0x4008_8FF8, 0x4009_7FF8, 0x4009_8FF8];
const FLEXCOMM_PSELID_I2S_TRANSMIT: u32 = 4;
const FLEXCOMM_PSELID_I2S_RECEIVE: u32 = 5;
const SYSCON_AHBCLKCTRL2: u32 = 0x4000_0208;
const SYSCON_AHBCLKCTRL2_USB1_DEV: u32 = 1 << 5;
const GPIO_DIR0: u32 = 0x4008_E000;
const GPIO_W0_28: u32 = 0x4008_D070;
const P0_28: u32 = 1 << 28;

pub const INHERITED: [InheritedItem; 16] = [
    InheritedItem {
        what: "the previous image enabled IRQ 4, 5, 10, 15 and 19 in ISER0 and IRQ 47 in ISER1",
        pokes: &[
            Poke::Word(
                NVIC_ISER0,
                (1 << PIN_INT0_IRQ)
                    | (1 << PIN_INT1_IRQ)
                    | (1 << CTIMER0_IRQ)
                    | (1 << FLEXCOMM1_IRQ)
                    | (1 << FLEXCOMM5_IRQ),
            ),
            Poke::Word(NVIC_ISER1, 1 << (USB1_IRQ - 32)),
        ],
    },
    InheritedItem {
        what: "priority 0x20 for the tick, USB and console interrupts",
        pokes: &[
            Poke::Byte(NVIC_IPR + CTIMER0_IRQ, INHERITED_PRIORITY),
            Poke::Byte(NVIC_IPR + USB1_IRQ, INHERITED_PRIORITY),
            Poke::Byte(NVIC_IPR + FLEXCOMM5_IRQ, INHERITED_PRIORITY),
        ],
    },
    InheritedItem {
        what: "the tick and USB interrupts are pending at resume, because the I2S frame clock and the USB microframes ran on while the core was halted for the load",
        pokes: &[
            Poke::Word(NVIC_ISPR0, 1 << CTIMER0_IRQ),
            Poke::Word(NVIC_ISPR1, 1 << (USB1_IRQ - 32)),
        ],
    },
    InheritedItem {
        what: "SYST_CSR 7, reload 100000, SysTick pending at resume for the same reason",
        pokes: &[
            Poke::Word(SYST_CSR, 7),
            Poke::Word(SYST_RVR, 100_000),
            Poke::Word(ICSR, ICSR_PENDSTSET),
        ],
    },
    InheritedItem {
        what: "VTOR at 0x20000000, the image region base where this image's vector table also loads",
        pokes: &[Poke::Word(VTOR, 0x2000_0000)],
    },
    InheritedItem {
        what: "DMA0 CTRL 1, SRAMBASE at the synthetic head table, channels 5, 11, 13, 16 and 18 enabled",
        pokes: &[
            Poke::Word(DMA0_CTRL, 1),
            Poke::Word(DMA0_SRAMBASE, SRAMBASE),
            Poke::Word(DMA0_ENABLESET0, INHERITED_ENABLED_CHANNELS),
        ],
    },
    InheritedItem {
        what: "the DMA0, CTIMER0 and four I2S Flexcomm clocks the running audio needs, SVD28 AHBCLKCTRL0 DMA0 bit 20, AHBCLKCTRL1 TIMER0 bit 26 and FC0, FC2, FC6, FC7 bits 11, 13, 17, 18",
        pokes: &[
            Poke::Word(SYSCON_AHBCLKCTRL0, SYSCON_AHBCLKCTRL0_DMA0),
            Poke::Word(
                SYSCON_AHBCLKCTRL1,
                SYSCON_AHBCLKCTRL1_TIMER0_AND_I2S_FLEXCOMMS,
            ),
        ],
    },
    InheritedItem {
        what: "FC0 and FC2 transmit I2S, FC6 and FC7 receive I2S, SVD28 PSELID PERSEL 4 and 5",
        pokes: &[
            Poke::Word(FLEXCOMM_PSELID[0], FLEXCOMM_PSELID_I2S_TRANSMIT),
            Poke::Word(FLEXCOMM_PSELID[1], FLEXCOMM_PSELID_I2S_TRANSMIT),
            Poke::Word(FLEXCOMM_PSELID[2], FLEXCOMM_PSELID_I2S_RECEIVE),
            Poke::Word(FLEXCOMM_PSELID[3], FLEXCOMM_PSELID_I2S_RECEIVE),
        ],
    },
    InheritedItem {
        what: "the channel 5 reload descriptor, linked to itself",
        pokes: &FC0_TX_DESCRIPTOR,
    },
    InheritedItem {
        what: "the channel 11 reload descriptor, linked to itself",
        pokes: &FC2_TX_DESCRIPTOR,
    },
    InheritedItem {
        what: "the channel 16 reload descriptor, linked to itself",
        pokes: &FC6_RX_DESCRIPTOR,
    },
    InheritedItem {
        what: "the channel 18 reload descriptor, linked to itself",
        pokes: &FC7_RX_DESCRIPTOR,
    },
    InheritedItem {
        what: "CTIMER0 TCR 1, the tick timer counting",
        pokes: &[Poke::Word(CTIMER0_TCR, 1)],
    },
    InheritedItem {
        what: "the four I2S Flexcomms enabled, SVD28 CFG1 MAINENABLE bit 0",
        pokes: &[
            Poke::Word(I2S_CFG1[0], I2S_CFG1_MAINENABLE),
            Poke::Word(I2S_CFG1[1], I2S_CFG1_MAINENABLE),
            Poke::Word(I2S_CFG1[2], I2S_CFG1_MAINENABLE),
            Poke::Word(I2S_CFG1[3], I2S_CFG1_MAINENABLE),
        ],
    },
    InheritedItem {
        what: "P0_28, the output-connect pin, driven high as an output, and the hand-over keeps it high",
        pokes: &[Poke::Word(GPIO_DIR0, P0_28), Poke::Word(GPIO_W0_28, 1)],
    },
    InheritedItem {
        what: "USB start-up: the device enabled and connected, SVD28 DEV_EN bit 7 and DCON bit 16, and the USB1 device clock that start-up turned on, SVD28 AHBCLKCTRL2 USB1_DEV bit 5",
        pokes: &[
            Poke::Word(USBHSD_DEVCMDSTAT, USBHSD_DEVCMDSTAT_DEV_EN_AND_DCON),
            Poke::Word(SYSCON_AHBCLKCTRL2, SYSCON_AHBCLKCTRL2_USB1_DEV),
        ],
    },
];

// WHY: Model choice, a stack pointer just under the stack top at 0x20030000; the image's own Reset reloads MSP
pub const INHERITED_STACK_POINTER: u32 = 0x2002_FE80;
