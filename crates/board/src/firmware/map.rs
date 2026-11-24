#[cfg(any(not(feature = "milestone4"), feature = "sound1"))]
use super::mmio::MemoryBytes;
use super::mmio::Register;

// WHY: SVD rows in fixtures/lpc55s28-svd-registers.txt, checked by crates/source-rules/tests/board_register_map.rs
pub const SYSCON_AHBCLKCTRLSET0: Register = Register::at(0x4000_0220);
pub const SYSCON_AHBCLKCTRL0_WWDT: u32 = 1 << 22;
pub const SYSCON_CLOCK_CTRL: Register = Register::at(0x4000_0A18);
pub const SYSCON_CLOCK_CTRL_FRO1MHZ_CLK_ENA: u32 = 1 << 6;
pub const SYSCON_WDTCLKDIV: Register = Register::at(0x4000_038C);
pub const SYSCON_WDTCLKDIV_RESET: u32 = 1 << 29;
// WHY: SDK fsl_clock.c CLOCK_SetClkDiv writes the divisor minus one, which also clears HALT
pub const SYSCON_WDTCLKDIV_DIVIDE_BY_ONE: u32 = 0;

pub const WWDT_MOD: Register = Register::at(0x4000_C000);
pub const WWDT_MOD_WDEN: u32 = 1 << 0;
pub const WWDT_MOD_WDRESET: u32 = 1 << 1;
pub const WWDT_MOD_WDPROTECT: u32 = 1 << 4;
pub const WWDT_TC: Register = Register::at(0x4000_C004);
pub const WWDT_FEED: Register = Register::at(0x4000_C008);
pub const WWDT_WARNINT: Register = Register::at(0x4000_C014);
pub const WWDT_WINDOW: Register = Register::at(0x4000_C018);
// WHY: SVD28 FEED description and SDK fsl_wwdt.c WWDT_Refresh
pub const WWDT_FEED_FIRST: u32 = 0xAA;
pub const WWDT_FEED_SECOND: u32 = 0x55;
// WHY: SDK fsl_wwdt.c WWDT_GetDefaultConfig, the SVD28 reset value of WINDOW, so no feed is ever too early
pub const WWDT_WINDOW_NONE: u32 = 0xFF_FFFF;
pub const WWDT_WARNINT_NONE: u32 = 0;

pub const DMA0_ENABLECLR0: Register = Register::at(0x4008_2028);
// WHY: I2S transmit runs on DMA0 channel 5 (FC0 TX) and channel 11 (FC2 TX), the board's factory channel assignment
pub const DMA0_FC0_TRANSMIT_CHANNEL: u32 = 1 << 5;
pub const DMA0_FC2_TRANSMIT_CHANNEL: u32 = 1 << 11;

pub const GPIO_W: Register = Register::at(0x4008_D000);
pub const GPIO_W_PORT_STEP: usize = 0x80;
pub const GPIO_W_PIN_STEP: usize = 0x4;
pub const GPIO_W_PORTS: u8 = 2;
pub const GPIO_W_PINS: u8 = 32;
pub const GPIO_DIRSET: Register = Register::at(0x4008_E380);
pub const GPIO_DIRSET_PORT_STEP: usize = 0x4;
pub const GPIO_CLR: Register = Register::at(0x4008_E280);
pub const GPIO_CLR_PORT_STEP: usize = 0x4;
// WHY: P0_28 is the output-connect pin; SVD28 CLR and DIRSET act on the bits written as ones
pub const GPIO_P0_28: u32 = 0x1000_0000;
pub const GPIO_P1_24: u32 = 0x0100_0000;
pub const GPIO_P1_25: u32 = 0x0200_0000;
// WHY: P1_4 is the phantom enable, high applies 48 V; the factory configuration boots it low as an output with the pad word IOCON_FACTORY_GPIO_OUTPUT_PAD
pub const GPIO_P1_4: u32 = 0x0000_0010;
// WHY: Device output_pin.rs, P1_3 is ConverterEnable and P1_29 AdcEnable, and the executor drives both with the pad word IOCON_FACTORY_GPIO_OUTPUT_PAD
pub const GPIO_P1_3: u32 = 0x0000_0008;
pub const GPIO_P1_29: u32 = 0x2000_0000;
// WHY: SVD28 PWORD, writing 0 clears the output bit and any other value sets it
pub const GPIO_W_LOW: u32 = 0;
pub const GPIO_W_HIGH: u32 = 1;

// WHY: CMSIS core_cm33.h DCB_BASE 0xE000EDF0 DEMCR +0x00C TRCENA bit 24, DWT_BASE 0xE0001000 CTRL +0x000 CYCCNTENA bit 0 NOCYCCNT bit 25, CYCCNT +0x004, fixtures/arm-core-debug-registers.md Core table
#[cfg(not(feature = "milestone4"))]
pub const DEMCR: Register = Register::at(0xE000_EDFC);
#[cfg(not(feature = "milestone4"))]
pub const DEMCR_TRCENA: u32 = 1 << 24;
#[cfg(not(feature = "milestone4"))]
pub const DWT_CTRL: Register = Register::at(0xE000_1000);
#[cfg(not(feature = "milestone4"))]
pub const DWT_CTRL_CYCCNTENA: u32 = 1 << 0;
#[cfg(not(feature = "milestone4"))]
pub const DWT_CTRL_NOCYCCNT: u32 = 1 << 25;
#[cfg(not(feature = "milestone4"))]
pub const DWT_CYCCNT: Register = Register::at(0xE000_1004);

// WHY: CMSIS core_cm33.h SCB at 0xE000ED00, AIRCR +0x00C, CCR +0x014, CFSR +0x028
pub const SCB_AIRCR: Register = Register::at(0xE000_ED0C);
pub const SCB_AIRCR_VECTKEY: u32 = 0x05FA << 16;
pub const SCB_AIRCR_PRIGROUP: u32 = 0x7 << 8;
pub const SCB_AIRCR_SYSRESETREQ: u32 = 1 << 2;
pub const SCB_CCR: Register = Register::at(0xE000_ED14);
pub const SCB_CCR_BFHFNMIGN: u32 = 1 << 8;
// WHY: Arm DDI 0553 Armv8-M ARM, CCR register, STKOFHFNMIGN bit 10 ignores stack limit violations at a requested priority below 0, so HardFault can push under MSPLIM instead of locking up
pub const SCB_CCR_STKOFHFNMIGN: u32 = 1 << 10;
pub const SCB_CFSR: Register = Register::at(0xE000_ED28);
pub const SCB_CFSR_PRECISERR: u32 = 1 << 9;
pub const SCB_CFSR_BFARVALID: u32 = 1 << 15;

// WHY: CMSIS core_cm33.h and mpu_armv8.h, core table
pub const MPU_TYPE: Register = Register::at(0xE000_ED90);
pub const MPU_TYPE_DREGION_SHIFT: u32 = 8;
pub const MPU_TYPE_DREGION_MASK: u32 = 0xFF;
pub const MPU_CTRL: Register = Register::at(0xE000_ED94);
pub const MPU_CTRL_ENABLE: u32 = 1 << 0;
pub const MPU_CTRL_OFF: u32 = 0;
pub const MPU_RNR: Register = Register::at(0xE000_ED98);
pub const MPU_RBAR: Register = Register::at(0xE000_ED9C);
pub const MPU_RLAR: Register = Register::at(0xE000_EDA0);
pub const MPU_MAIR0: Register = Register::at(0xE000_EDC0);
pub const MPU_RBAR_XN: u32 = 1 << 0;
pub const MPU_RBAR_AP_READ_WRITE_PRIVILEGED: u32 = 0b00 << 1;
pub const MPU_RBAR_SH_NON_SHAREABLE: u32 = 0b00 << 3;
pub const MPU_RLAR_EN: u32 = 1 << 0;
pub const MPU_RLAR_ATTRINDX_SHIFT: u32 = 1;
pub const MPU_ADDRESS_GRANULE: u32 = 32;
pub const MPU_RLAR_DISABLED: u32 = 0;
// WHY: Mpu_armv8.h ARM_MPU_ATTR(ARM_MPU_ATTR_MEMORY_(1, 1, 1, 1), same), ARM_MPU_ATTR(ARM_MPU_ATTR_DEVICE, ARM_MPU_ATTR_DEVICE_nGnRE) and ARM_MPU_ATTR(ARM_MPU_ATTR_NON_CACHEABLE, same)
pub const MAIR_NORMAL_WRITE_BACK: u32 = 0xFF;
pub const MAIR_DEVICE_NGNRE: u32 = 0x04;
pub const MAIR_NORMAL_NON_CACHEABLE: u32 = 0x44;

// WHY: The factory page holds a magic word at 0x38200 and ten serial bytes at 0x38204
#[cfg(any(not(feature = "milestone4"), feature = "sound1"))]
pub const FACTORY_MAGIC: MemoryBytes = MemoryBytes::at(0x0003_8200);
#[cfg(any(not(feature = "milestone4"), feature = "sound1"))]
pub const FACTORY_SERIAL: MemoryBytes = MemoryBytes::at(0x0003_8204);
// WHY: NXP fsl_power.c FLASH_NMPA_DCDC_POWER_PROFILE_HIGH_0/1_ADDRS and FLASH_NMPA_PVT_MONITOR_0/1_RINGO_ADDRS, read once at the loader's clock by sound3
#[cfg(feature = "sound3")]
pub const NMPA_WORDS: [MemoryBytes; 4] = [
    MemoryBytes::at(0x0009_FCD8),
    MemoryBytes::at(0x0009_FCDC),
    MemoryBytes::at(0x0009_FD30),
    MemoryBytes::at(0x0009_FD40),
];

// WHY: SVD28 AHBCLKCTRL array of three at 0x200 step 4, read-write, the enable state a reader sees
pub const SYSCON_AHBCLKCTRL0: Register = Register::at(0x4000_0200);
pub const SYSCON_AHBCLKCTRL2: Register = Register::at(0x4000_0208);
// WHY: SVD28 AHBCLKCTRLSET array of three, written bit by bit as SDK fsl_clock.c CLOCK_EnableClock does
pub const SYSCON_AHBCLKCTRLSET_INDEX_STEP: usize = 0x4;
pub const SYSCON_AHBCLKCTRL0_IOCON: u32 = 1 << 13;
pub const SYSCON_AHBCLKCTRL0_GPIO0: u32 = 1 << 14;
pub const SYSCON_AHBCLKCTRL0_GPIO1: u32 = 1 << 15;
pub const SYSCON_AHBCLKCTRL0_DMA0: u32 = 1 << 20;
pub const SYSCON_AHBCLKCTRL1_FC0: u32 = 1 << 11;
pub const SYSCON_AHBCLKCTRL1_FC1: u32 = 1 << 12;
pub const SYSCON_AHBCLKCTRL1_FC2: u32 = 1 << 13;
pub const SYSCON_AHBCLKCTRL1_FC3: u32 = 1 << 14;
pub const SYSCON_AHBCLKCTRL1_FC6: u32 = 1 << 17;
pub const SYSCON_AHBCLKCTRL1_FC7: u32 = 1 << 18;
pub const SYSCON_AHBCLKCTRL1_TIMER0: u32 = 1 << 26;
pub const SYSCON_AHBCLKCTRL2_USB1_DEV: u32 = 1 << 5;

pub const SYSCON_PRESETCTRL1: Register = Register::at(0x4000_0104);
pub const SYSCON_PRESETCTRL1_FC3_RST: u32 = 1 << 14;

pub const SYSCON_FCCLKSEL1: Register = Register::at(0x4000_02B4);
pub const SYSCON_FCCLKSEL3: Register = Register::at(0x4000_02BC);
// WHY: The SPI and I2C Flexcomms take clock source 3, SVD28 enumeration 3 is FRO 96 MHz
pub const SYSCON_FCCLKSEL_FACTORY_SERIAL_BUS_SOURCE: u32 = 3;

pub const SYSCON_MAINCLKSELA: Register = Register::at(0x4000_0280);
pub const SYSCON_MAINCLKSELB: Register = Register::at(0x4000_0284);
pub const SYSCON_AHBCLKDIV: Register = Register::at(0x4000_0380);
pub const SYSCON_FROHFDIV: Register = Register::at(0x4000_0388);
// WHY: The main clock runs from PLL1 on CLKIN, SVD28 enumerations MAINCLKSELB 2 is PLL1 and MAINCLKSELA 1 is CLKIN
pub const SYSCON_AHBCLKDIV_DIVIDE_BY_ONE: u32 = 0;
pub const SYSCON_MAINCLKSELB_SEL_PLL1: u32 = 2;
pub const SYSCON_MAINCLKSELA_SEL_CLKIN: u32 = 1;
// WHY: UM11126 rev 1.8 Table 72, MAINCLKSELA 3 is the FRO 96 MHz clock
pub const SYSCON_MAINCLKSELA_SEL_FRO_96MHZ: u32 = 3;
pub const SYSCON_FROHFDIV_FACTORY_DIVIDE_BY_FOUR: u32 = 3;
// WHY: UM11126 rev 1.8 Table 72 and 73, MAINCLKSELB 0 passes main_clk_a, and MAINCLKSELA 0 is the FRO 12 MHz; read back for the console only
pub const SYSCON_MAINCLKSELB_SEL_MAIN_A: u32 = 0;
pub const SYSCON_MAINCLKSELA_SEL_FRO_12MHZ: u32 = 0;
// WHY: UM11126 rev 1.8 FROHFDIV, DIV is bits 7:0; bits 29 to 31 are RESET, HALT and REQFLAG
pub const SYSCON_FROHFDIV_DIV: u32 = 0xFF;
pub const SYSCON_FMCCR: Register = Register::at(0x4000_0400);
// WHY: UM11126 rev 1.8 Table 113, a flash access takes FLASHTIM + 1 system clocks and 8 is nine clocks, good to 100 MHz; SVD28 and SYSCON_FMCCR_FLASHTIM_MASK 0xF000, the field is bits 15:12
pub const SYSCON_FMCCR_FLASHTIM: u32 = 0xF << 12;
pub const SYSCON_FMCCR_FLASHTIM_UP_TO_100_MHZ: u32 = 8 << 12;

pub const SYSCON_PLL0CLKSEL: Register = Register::at(0x4000_0290);
pub const SYSCON_PLL0CTRL: Register = Register::at(0x4000_0580);
pub const SYSCON_PLL0NDEC: Register = Register::at(0x4000_0588);
pub const SYSCON_PLL0PDEC: Register = Register::at(0x4000_058C);
pub const SYSCON_PLL0STAT: Register = Register::at(0x4000_0584);
// WHY: LPC55S28.h SYSCON_PLL0STAT_LOCK_MASK and SYSCON_PLL1STAT_LOCK_MASK, LOCK is bit 0 of both
pub const SYSCON_PLL0STAT_LOCK: u32 = 1 << 0;
pub const SYSCON_PLL0SSCG1: Register = Register::at(0x4000_0594);
// WHY: The same writes for both families, M through PLL0SSCG1 with SEL_EXT, no request bit
pub const SYSCON_PLL0CLKSEL_SEL_CLKIN: u32 = 1;
// WHY: The factory PLL0CTRL word on a part whose PLL0CTRL holds its SVD28 reset value 0, written whole so no boot ROM leftover survives
pub const SYSCON_PLL0CTRL_FACTORY_WORD_FROM_RESET: u32 = 0x0020_7C30;
pub const SYSCON_PLL0NDEC_FACTORY_N: u32 = 125;
pub const SYSCON_PLL0PDEC_FACTORY_P: u32 = 2;
pub const SYSCON_PLL0SSCG1_48_KHZ_FAMILY_M_2048: u32 = 0x1020_0000;
pub const SYSCON_PLL0SSCG1_44_1_KHZ_FAMILY_M_2352: u32 = 0x1024_C000;

pub const SYSCON_MCLKDIV: Register = Register::at(0x4000_03AC);
pub const SYSCON_MCLKDIV_HALT: u32 = 1 << 30;
pub const SYSCON_PLL0CLKDIV: Register = Register::at(0x4000_03C4);
pub const SYSCON_PLL0CLKDIV_HALT: u32 = 1 << 30;
// WHY: DIV 3 divides by four for 48 kHz and DIV 4 by five for 44.1 kHz
pub const AUDIO_CLOCK_DIV_48_KHZ_FAMILY: u32 = 3;
pub const AUDIO_CLOCK_DIV_44_1_KHZ_FAMILY: u32 = 4;

pub const SYSCON_MCLKCLKSEL: Register = Register::at(0x4000_02E0);
// WHY: The audio Flexcomms take clock source 1, SVD28 enumeration 1 is PLL0
pub const SYSCON_MCLKCLKSEL_SEL_PLL0: u32 = 1;
pub const SYSCON_MCLKIO: Register = Register::at(0x4000_0420);
pub const SYSCON_MCLKIO_MCLKIO: u32 = 1 << 0;
pub const IOCON_PIO1_31: Register = Register::at(0x4000_10FC);
// WHY: P1_31 carries MCLK with the factory pad word 0x101, FUNC 1 with DIGIMODE
pub const IOCON_PIO1_31_FACTORY_MCLK: u32 = 0x101;
pub const IOCON_PIO1_28: Register = Register::at(0x4000_10F0);
pub const IOCON_PIO1_28_FACTORY_FC7_SCK: u32 = 0x101;
pub const IOCON_PIO1_30: Register = Register::at(0x4000_10F8);
pub const IOCON_PIO1_30_FACTORY_FC7_WS: u32 = 0x101;
pub const IOCON_PIO0_20: Register = Register::at(0x4000_1050);
pub const IOCON_PIO0_20_FACTORY_FC7_DATA: u32 = 0x107;
pub const IOCON_PIO1_12: Register = Register::at(0x4000_10B0);
pub const IOCON_PIO1_12_FACTORY_FC6_SCK: u32 = 0x102;
pub const IOCON_PIO1_13: Register = Register::at(0x4000_10B4);
pub const IOCON_PIO1_13_FACTORY_FC6_DATA: u32 = 0x102;
pub const IOCON_PIO0_26: Register = Register::at(0x4000_1068);
pub const IOCON_PIO0_26_FACTORY_FC2_DATA: u32 = 0x101;
pub const IOCON_PIO0_24: Register = Register::at(0x4000_1060);
pub const IOCON_PIO0_24_FACTORY_FC0_DATA: u32 = 0x101;

// WHY: Stopping the audio clears DMA0 channels 5, 11, 16 and 18 with ENABLECLR0 = 0x50820
pub const DMA0_FC6_RECEIVE_CHANNEL: u32 = 1 << 16;
pub const DMA0_FC7_RECEIVE_CHANNEL: u32 = 1 << 18;

pub const CTIMER0_TCR: Register = Register::at(0x4000_8004);
// WHY: Stopping the tick timer writes TCR = 0, SVD28 CEN and CRST clear
pub const CTIMER0_TCR_STOPPED: u32 = 0;

pub const I2S7_CFG1: Register = Register::at(0x4009_8C00);
pub const I2S6_CFG1: Register = Register::at(0x4009_7C00);
pub const I2S2_CFG1: Register = Register::at(0x4008_8C00);
pub const I2S0_CFG1: Register = Register::at(0x4008_6C00);
// WHY: Stopping the audio writes CFG1 = 0 on FC7, FC6, FC2 and FC0, in that order
pub const I2S_CFG1_DISABLED: u32 = 0;

pub const USBHSD_DEVCMDSTAT: Register = Register::at(0x4009_4000);
pub const USBHSD_DEVCMDSTAT_SETUP: u32 = 1 << 8;
pub const USBHSD_DEVCMDSTAT_FORCE_NEEDCLK: u32 = 1 << 9;
pub const USBHSD_DEVCMDSTAT_DCON: u32 = 1 << 16;
pub const USBHSD_DEVCMDSTAT_DCON_C: u32 = 1 << 24;
pub const USBHSD_DEVCMDSTAT_DSUS_C: u32 = 1 << 25;
pub const USBHSD_DEVCMDSTAT_DRES_C: u32 = 1 << 26;
pub const USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR: u32 = USBHSD_DEVCMDSTAT_SETUP
    | USBHSD_DEVCMDSTAT_DCON_C
    | USBHSD_DEVCMDSTAT_DSUS_C
    | USBHSD_DEVCMDSTAT_DRES_C;

// WHY: SVD28 rows and UM11126 Tables 834 to 845, the USB1 device registers the driver reads and writes
pub const USBHSD_DEVCMDSTAT_DEV_ADDR_MASK: u32 = 0x7F;
pub const USBHSD_DEVCMDSTAT_DEV_EN: u32 = 1 << 7;
pub const USBHSD_DEVCMDSTAT_LPM_SUP: u32 = 1 << 11;
pub const USBHSD_DEVCMDSTAT_DSUS: u32 = 1 << 17;
pub const USBHSD_DEVCMDSTAT_SPEED_SHIFT: u32 = 22;
pub const USBHSD_DEVCMDSTAT_SPEED_MASK: u32 = 0x3;
// WHY: UM11126 Table 834 Speed field, 01b full-speed, 10b high-speed, 00b and 11b reserved
pub const USBHSD_DEVCMDSTAT_SPEED_HIGH: u32 = 0b10;
pub const USBHSD_EPLISTSTART: Register = Register::at(0x4009_4008);
pub const USBHSD_EPSKIP: Register = Register::at(0x4009_4014);
pub const USBHSD_EPINUSE: Register = Register::at(0x4009_4018);
pub const USBHSD_EPBUFCFG: Register = Register::at(0x4009_401C);
pub const USBHSD_INTSTAT: Register = Register::at(0x4009_4020);
pub const USBHSD_INTSTAT_EP0OUT: u32 = 1 << 0;
pub const USBHSD_INTSTAT_EP0IN: u32 = 1 << 1;
pub const USBHSD_INTSTAT_FRAME_INT: u32 = 1 << 30;
pub const USBHSD_INTSTAT_DEV_INT: u32 = 1 << 31;
pub const USBHSD_INTEN: Register = Register::at(0x4009_4024);

// WHY: UM11126 rev 1.8 section 44.1 and Table 6, USB RAM is its own AHB slave at 0x4010_0000 to 0x4010_3FFF
pub const USB1_RAM_FIRST_BYTE: u32 = 0x4010_0000;
// WHY: UM11126 Table 845 and Fig 163 endpoint list word, bit positions from the USBD register description; the list sits at the start of USB RAM, and the attach writes that address to EPLISTSTART; Table 819 fixes bits 31:20 at 0x401
pub const USB1_RAM_ENDPOINT_LIST: Register = Register::at(0x4010_0000);
pub const USB1_RAM_LIST_ACTIVE: u32 = 1 << 31;
pub const USB1_RAM_LIST_DISABLED: u32 = 1 << 30;
pub const USB1_RAM_LIST_STALL: u32 = 1 << 29;
pub const USB1_RAM_LIST_TOGGLE_RESET: u32 = 1 << 28;
pub const USB1_RAM_LIST_PERIODIC: u32 = 1 << 26;
pub const USB1_RAM_LIST_NBYTES_SHIFT: u32 = 11;
pub const USB1_RAM_LIST_NBYTES_MASK: u32 = 0x7FFF;
pub const USB1_RAM_LIST_ADDRESS_OFFSET_SHIFT: u32 = 6;
pub const USB1_RAM_LIST_ADDRESS_OFFSET_MASK: u32 = 0x7FF;
pub const USB1_RAM_LIST_RESTING: u32 = 0;

// WHY: SVD28 AHBCLKCTRL2 fields; these clocks are set with ANALOG_CTRL before the USB start-up
pub const SYSCON_AHBCLKCTRL2_USB1_HOST: u32 = 1 << 4;
pub const SYSCON_AHBCLKCTRL2_USB1_RAM: u32 = 1 << 6;
pub const SYSCON_AHBCLKCTRL2_USB1_PHY: u32 = 1 << 7;
// WHY: The factory power word 0x009C17C0 and XO32M_CTRL |= 0x01800000 power the USB PHY, its LDO and the crystal output to the PHY PLL, SVD28 fields
pub const PMC_PDRUNCFG0_PDEN_USBHSPHY: u32 = 1 << 12;
pub const PMC_PDRUNCFG0_PDEN_LDOUSBHS: u32 = 1 << 18;
pub const ANACTRL_XO32M_CTRL_ENABLE_PLL_USB_OUT: u32 = 1 << 23;
pub const USBHSH_PORTMODE: Register = Register::at(0x400A_3050);
pub const USBHSH_PORTMODE_DEV_ENABLE: u32 = 1 << 16;
pub const USBPHY_PWD: Register = Register::at(0x4003_8000);
pub const USBPHY_CTRL_SET: Register = Register::at(0x4003_8034);
pub const USBPHY_CTRL_SET_ENAUTOCLR_CLKGATE: u32 = 1 << 19;
pub const USBPHY_CTRL_SET_ENAUTOCLR_PHY_PWD: u32 = 1 << 20;
pub const USBPHY_CTRL_CLR: Register = Register::at(0x4003_8038);
pub const USBPHY_CTRL_CLR_CLKGATE: u32 = 1 << 30;
pub const USBPHY_CTRL_CLR_SFTRST: u32 = 1 << 31;
pub const USBPHY_PLL_SIC: Register = Register::at(0x4003_80A0);
// WHY: UM11126 Rev 1.3 Table 826 and SVD28, PLL_LOCK is read-only; no NXP sequence reads it, so its answer is reported but never gates the connect
pub const USBPHY_PLL_SIC_PLL_LOCK: u32 = 1 << 31;
pub const USBPHY_USB1_VBUS_DETECT: Register = Register::at(0x4003_80C0);
pub const USBPHY_USB1_VBUS_DETECT_VBUS_OVERRIDE_EN: u32 = 1 << 3;
pub const USBPHY_USB1_VBUS_DETECT_BVALID_OVERRIDE: u32 = 1 << 5;
pub const USBPHY_USB1_VBUS_DETECT_VBUSVALID_OVERRIDE: u32 = 1 << 7;
// WHY: PLL_SIC 0x00A03040 is PLL_EN_USB_CLKS, PLL_POWER, PLL_ENABLE, PLL_REG_ENABLE and PLL_DIV_SEL 2, SVD28, and PWD 0 powers every PHY block
pub const USBPHY_PLL_SIC_FACTORY: u32 = 0x00A0_3040;
pub const USBPHY_PWD_EVERYTHING_POWERED: u32 = 0;

pub const FLEXCOMM3_PSELID: Register = Register::at(0x4008_9FF8);
// WHY: SVD28 PERSEL 3 is I2C, selected on Flexcomm3 for the converter bus
pub const FLEXCOMM_PSELID_I2C: u32 = 3;
pub const I2C3_CFG: Register = Register::at(0x4008_9800);
pub const I2C3_CFG_MSTEN: u32 = 1 << 0;
pub const I2C3_CFG_MASTER_OFF: u32 = 0;
pub const I2C3_STAT: Register = Register::at(0x4008_9804);
pub const I2C3_STAT_MSTPENDING: u32 = 1 << 0;
pub const I2C3_STAT_MSTARBLOSS: u32 = 1 << 4;
pub const I2C3_STAT_MSTSTSTPERR: u32 = 1 << 6;
pub const I2C_STAT_MSTSTATE_SHIFT: u32 = 1;
pub const I2C_STAT_MSTSTATE_MASK: u32 = 0x7;
// WHY: SVD28 MSTSTATE enumeration
pub const I2C_MSTSTATE_IDLE: u32 = 0;
pub const I2C_MSTSTATE_RECEIVE_READY: u32 = 1;
pub const I2C_MSTSTATE_TRANSMIT_READY: u32 = 2;
pub const I2C_MSTSTATE_NACK_ADDRESS: u32 = 3;
pub const I2C_MSTSTATE_NACK_DATA: u32 = 4;
pub const I2C3_CLKDIV: Register = Register::at(0x4008_9814);
pub const I2C3_MSTTIME: Register = Register::at(0x4008_9824);
// WHY: Flexcomm3 CLKDIV 0x0E and MSTTIME 0, the factory I2C timing
pub const I2C3_CLKDIV_FACTORY: u32 = 0x0E;
pub const I2C3_MSTTIME_FACTORY: u32 = 0;
pub const I2C3_MSTCTL: Register = Register::at(0x4008_9820);
pub const I2C3_MSTCTL_MSTCONTINUE: u32 = 1 << 0;
pub const I2C3_MSTCTL_MSTSTART: u32 = 1 << 1;
pub const I2C3_MSTCTL_MSTSTOP: u32 = 1 << 2;
pub const I2C3_MSTDAT: Register = Register::at(0x4008_9828);
pub const I2C_MSTDAT_DATA_MASK: u32 = 0xFF;

pub const SPI1_FIFOSTAT: Register = Register::at(0x4008_7E04);
pub const SPI1_FIFOSTAT_TXNOTFULL: u32 = 1 << 5;
pub const SPI1_FIFOWR: Register = Register::at(0x4008_7E20);
// WHY: The word with SSEL1 held, the shift byte with EOT, then eight clocks with no select
pub const SPI1_FIFOWR_FACTORY_WORD_FRAME: u32 = 0x0F4D_0000;
pub const SPI1_FIFOWR_FACTORY_SHIFT_BYTE_FRAME: u32 = 0x075D_0000;
pub const SPI1_FIFOWR_FACTORY_PAD_FRAME: u32 = 0x075F_0000;

// WHY: CMSIS core_cm33.h NVIC at 0xE000E100, ICER +0x080 and ICPR +0x180, one bit per interrupt
pub const NVIC_ICER: Register = Register::at(0xE000_E180);
pub const NVIC_ICPR: Register = Register::at(0xE000_E280);
pub const NVIC_WORD_BYTES: usize = 4;
pub const NVIC_EVERY_INTERRUPT_IN_A_WORD: u32 = 0xFFFF_FFFF;
// WHY: CMSIS core_cm33.h NVIC ISER +0x000 and IPR +0x300, one byte per interrupt; USB1 is IRQ47, at priority 0x20 in the factory configuration
pub const NVIC_ISER: Register = Register::at(0xE000_E100);
pub const NVIC_IPR: Register = Register::at(0xE000_E400);
pub const NVIC_USB1_IRQ: usize = 47;
pub const NVIC_USB1_FACTORY_PRIORITY: u32 = 0x20;

// WHY: CMSIS core_cm33.h SysTick, core table
pub const SYST_CSR: Register = Register::at(0xE000_E010);
pub const SYST_CSR_ENABLE: u32 = 1 << 0;
pub const SYST_CSR_CLKSOURCE: u32 = 1 << 2;
pub const SYST_CSR_COUNTFLAG: u32 = 1 << 16;
pub const SYST_RVR: Register = Register::at(0xE000_E014);
pub const SYST_CVR: Register = Register::at(0xE000_E018);
// WHY: SysTick reload 100000 gives a 1 ms timer at the 100 MHz core clock
pub const SYST_RVR_FACTORY_ONE_MILLISECOND: u32 = 100_000;

// WHY: These are powered before any divider is set, SVD28 PDRUNCFG0 fields
pub const PMC_PDRUNCFGCLR0: Register = Register::at(0x4002_00C8);
#[cfg(feature = "sound3")]
pub const PMC_PDRUNCFGSET0: Register = Register::at(0x4002_00C0);
// WHY: LPC55S28.h PMC_Type, DCDC0 at offset 0x10, DCDC1 at 0x14 and LDOPMU at 0x1C; read only, by the power report
pub const PMC_DCDC0: Register = Register::at(0x4002_0010);
pub const PMC_DCDC1: Register = Register::at(0x4002_0014);
pub const PMC_LDOPMU: Register = Register::at(0x4002_001C);
pub const PMC_PDRUNCFG0_PDEN_XTAL32M: u32 = 1 << 8;
pub const PMC_PDRUNCFG0_PDEN_PLL0: u32 = 1 << 9;
pub const PMC_PDRUNCFG0_PDEN_PLL1: u32 = 1 << 10;
pub const PMC_PDRUNCFG0_PDEN_LDOXO32M: u32 = 1 << 20;
pub const PMC_PDRUNCFG0_PDEN_PLL0_SSCG: u32 = 1 << 23;
pub const PMC_RTCOSC32K: Register = Register::at(0x4002_0098);
pub const PMC_RTCOSC32K_FACTORY_WORD: u32 = 0;
pub const PMC_PDRUNCFG0_PDEN_FRO32K: u32 = 1 << 6;
pub const PMC_PDRUNCFG0_PDEN_XTAL32K: u32 = 1 << 7;

// WHY: The analog control block is clocked before use, SVD28 AHBCLKCTRL2 resets to 0
pub const SYSCON_AHBCLKCTRL2_ANALOG_CTRL: u32 = 1 << 27;
pub const ANACTRL_FRO192M_CTRL: Register = Register::at(0x4001_3010);
pub const ANACTRL_FRO192M_CTRL_ENA_96MHZCLK: u32 = 1 << 30;
pub const ANACTRL_XO32M_CTRL: Register = Register::at(0x4001_3020);
pub const ANACTRL_XO32M_CTRL_ENABLE_SYSTEM_CLK_OUT: u32 = 1 << 24;
pub const ANACTRL_XO32M_STATUS: Register = Register::at(0x4001_3024);
// WHY: SDK fsl_clock.c CLOCK_SetupExtClocking waits for XO_READY before clk_in is used
pub const ANACTRL_XO32M_STATUS_XO_READY: u32 = 1 << 0;
pub const SYSCON_CLOCK_CTRL_CLKIN_ENA: u32 = 1 << 5;
pub const SYSCON_CLOCK_CTRL_XO_CAL_CLK_ENA: u32 = 1 << 8;

pub const SYSCON_PLL1CLKSEL: Register = Register::at(0x4000_0294);
pub const SYSCON_PLL1CTRL: Register = Register::at(0x4000_0560);
pub const SYSCON_PLL1NDEC: Register = Register::at(0x4000_0568);
pub const SYSCON_PLL1MDEC: Register = Register::at(0x4000_056C);
pub const SYSCON_PLL1PDEC: Register = Register::at(0x4000_0570);
pub const SYSCON_PLL1STAT: Register = Register::at(0x4000_0564);
// WHY: LPC55S28.h SYSCON_PLL1STAT_LOCK_MASK, LOCK is bit 0
pub const SYSCON_PLL1STAT_LOCK: u32 = 1 << 0;
// WHY: PLL1 from clk_in at N 3, M 25, P 1, no request bit set
pub const SYSCON_PLL1CLKSEL_SEL_CLKIN: u32 = 1;
pub const SYSCON_PLL1NDEC_FACTORY_N: u32 = 3;
pub const SYSCON_PLL1MDEC_FACTORY_M: u32 = 25;
pub const SYSCON_PLL1PDEC_FACTORY_P: u32 = 1;
// WHY: SELI 15, SELP 7, CLKEN 1, every other bit at its SVD28 reset value 0, so SELR 0 as pllFindSel sets it
pub const SYSCON_PLL1CTRL_FACTORY_WORD_FROM_RESET: u32 = 0x0020_1CF0;

pub const SYSCON_PRESETCTRL1_FC1_RST: u32 = 1 << 12;
pub const FLEXCOMM1_PSELID: Register = Register::at(0x4008_7FF8);
// WHY: Flexcomm1 runs SPI for the panel bus, SVD28 PERSEL 2 is SPI
pub const FLEXCOMM_PSELID_SPI: u32 = 2;
pub const SPI1_CFG: Register = Register::at(0x4008_7400);
// WHY: ENABLE, MASTER, CPHA 1, CPOL 1, MSB first, every select active low
pub const SPI1_CFG_FACTORY_MODE_3_MASTER: u32 = 0x35;
pub const SPI1_DIV: Register = Register::at(0x4008_7424);
// WHY: DIVVAL 0x0E divides the Flexcomm clock by 15
pub const SPI1_DIV_FACTORY: u32 = 0x0E;
pub const SPI1_FIFOCFG: Register = Register::at(0x4008_7E00);
pub const SPI1_FIFOCFG_ENABLETX: u32 = 1 << 0;
pub const SPI1_FIFOCFG_EMPTYTX: u32 = 1 << 16;
pub const SPI1_FIFOCFG_EMPTYRX: u32 = 1 << 17;
pub const SPI1_FIFOSTAT_TXEMPTY: u32 = 1 << 4;
pub const SPI1_STAT: Register = Register::at(0x4008_7408);
pub const SPI1_STAT_MSTIDLE: u32 = 1 << 8;

// WHY: Factory IOCON words, LPC55S6x data sheet function 4 FC1_SCK, 5 FC1 MOSI, 1 FC1 SSEL1, with EGP and FILTEROFF set on the two I2C pads
pub const IOCON_PIO0_7: Register = Register::at(0x4000_101C);
pub const IOCON_PIO0_7_FACTORY_FC1_SCK: u32 = 0x104;
pub const IOCON_PIO0_13: Register = Register::at(0x4000_1034);
pub const IOCON_PIO0_13_FACTORY_FC1_MOSI: u32 = 0x5105;
pub const IOCON_PIO0_14: Register = Register::at(0x4000_1038);
pub const IOCON_PIO0_14_FACTORY_FC1_SSEL1: u32 = 0x5101;

// WHY: Factory IOCON words, data sheet function 1, pull-up, DIGIMODE, OD
pub const IOCON_PIO0_3: Register = Register::at(0x4000_100C);
pub const IOCON_PIO0_3_FACTORY_FC3_SDA: u32 = 0x321;
pub const IOCON_PIO0_21: Register = Register::at(0x4000_1054);
pub const IOCON_PIO0_21_FACTORY_FC3_SCL: u32 = 0x321;

// WHY: Every driven pin is a GPIO output, FUNC 0 with DIGIMODE and no pull, as in the factory configuration
pub const IOCON_PIO0_28: Register = Register::at(0x4000_1070);
pub const IOCON_PIO1_3: Register = Register::at(0x4000_108C);
pub const IOCON_PIO1_4: Register = Register::at(0x4000_1090);
pub const IOCON_PIO1_10: Register = Register::at(0x4000_10A8);
pub const IOCON_PIO1_22: Register = Register::at(0x4000_10D8);
// WHY: IOCON_PIO1_25 and then IOCON_PIO1_24 take 0x100, FUNC 0 with DIGIMODE and no pull; SVD28 PIO[2][32] steps 0x80 a port and 0x4 a pin from base 0x40001000
pub const IOCON_PIO1_24: Register = Register::at(0x4000_10E0);
pub const IOCON_PIO1_25: Register = Register::at(0x4000_10E4);
pub const IOCON_PIO1_26: Register = Register::at(0x4000_10E8);
pub const IOCON_PIO1_29: Register = Register::at(0x4000_10F4);
pub const IOCON_FACTORY_GPIO_OUTPUT_PAD: u32 = 0x100;

pub const SYSCON_AHBCLKCTRL0_MUX: u32 = 1 << 11;
pub const SYSCON_AHBCLKCTRL0_ADC: u32 = 1 << 27;
pub const SYSCON_AHBCLKCTRL1_FC4: u32 = 1 << 15;
pub const SYSCON_PRESETCTRL0: Register = Register::at(0x4000_0100);
pub const SYSCON_PRESETCTRL0_ADC_RST: u32 = 1 << 27;
pub const SYSCON_PRESETCTRL1_FC4_RST: u32 = 1 << 15;
pub const SYSCON_FCCLKSEL4: Register = Register::at(0x4000_02C0);
// WHY: The factory PDRUNCFGCLR0 word 0x009C17C0 powers AUXBIAS, which the ADC is taken to need
pub const PMC_PDRUNCFG0_PDEN_AUXBIAS: u32 = 1 << 19;
pub const SYSCON_ADCCLKSEL: Register = Register::at(0x4000_02A4);
pub const SYSCON_ADCCLKSEL_FACTORY_FRO_96_MHZ: u32 = 2;
pub const SYSCON_ADCCLKDIV: Register = Register::at(0x4000_0394);
pub const SYSCON_ADCCLKDIV_FACTORY_DIVIDE_BY_SIX: u32 = 5;

// WHY: Factory IOCON words for the panel, knob and jack pins
pub const IOCON_PIO0_1: Register = Register::at(0x4000_1004);
pub const IOCON_PIO0_2: Register = Register::at(0x4000_1008);
pub const IOCON_PIO0_4: Register = Register::at(0x4000_1010);
pub const IOCON_PIO0_5: Register = Register::at(0x4000_1014);
pub const IOCON_PIO0_10: Register = Register::at(0x4000_1028);
pub const IOCON_PIO0_15: Register = Register::at(0x4000_103C);
pub const IOCON_PIO0_16: Register = Register::at(0x4000_1040);
pub const IOCON_PIO0_18: Register = Register::at(0x4000_1048);
pub const IOCON_PIO0_19: Register = Register::at(0x4000_104C);
pub const IOCON_PIO0_23: Register = Register::at(0x4000_105C);
pub const IOCON_PIO0_31: Register = Register::at(0x4000_107C);
pub const IOCON_PIO1_6: Register = Register::at(0x4000_1098);
pub const IOCON_PIO1_15: Register = Register::at(0x4000_10BC);
pub const IOCON_FACTORY_DIGITAL_GPIO: u32 = 0x100;
pub const IOCON_FACTORY_PULLED_UP_INPUT: u32 = 0x120;
pub const IOCON_FACTORY_FC4_SELECT: u32 = 0x101;
pub const IOCON_FACTORY_FC4_CLOCK_AND_DATA: u32 = 0x102;
pub const IOCON_FACTORY_ANALOG_INPUT: u32 = 0x400;

pub const GPIO_DIRCLR: Register = Register::at(0x4008_E400);
pub const GPIO_DIRCLR_PORT_STEP: usize = 0x4;
pub const GPIO_DIRCLR0_FACTORY_KNOB_INPUTS: u32 = 0x8081_8400;
// WHY: SVD28 PWORD, a word pin reads 0 when the pin is low and all ones when it is high
pub const GPIO_W_READ_LOW: u32 = 0;

pub const FLEXCOMM4_PSELID: Register = Register::at(0x4008_AFF8);
pub const SPI4_CFG: Register = Register::at(0x4008_A400);
pub const SPI4_CFG_FACTORY_MASTER_MODE_0: u32 = 5;
pub const SPI4_DIV: Register = Register::at(0x4008_A424);
pub const SPI4_DIV_FACTORY_DIVIDE_BY_TWO: u32 = 1;
// WHY: SVD28 FIFOCFG fields; the FIFO is flushed, then left at 0x1001, ENABLETX and DMATX
pub const SPI4_FIFOCFG: Register = Register::at(0x4008_AE00);
pub const SPI4_FIFOCFG_ENABLETX: u32 = 1 << 0;
pub const SPI4_FIFOCFG_DMATX: u32 = 1 << 12;
pub const SPI4_FIFOCFG_EMPTYTX: u32 = 1 << 16;
pub const SPI4_FIFOCFG_EMPTYRX: u32 = 1 << 17;
pub const SPI4_FIFOSTAT: Register = Register::at(0x4008_AE04);
pub const SPI4_FIFOSTAT_TXEMPTY: u32 = 1 << 4;
pub const SPI4_FIFOWR: Register = Register::at(0x4008_AE20);

// WHY: Request 13 and DMA0 CTRL 1, the Non-secure alias of 0x50006748, UM11126 section 2.1.3
pub const INPUTMUX_DMA0_REQ_ENA_SET: Register = Register::at(0x4000_6748);
// WHY: FC7, FC6, FC2 and FC0 take the PLL0 divided clock and FC7's shared SCK and WS, SVD28 rows
pub const SYSCON_FCCLKSEL0: Register = Register::at(0x4000_02B0);
pub const SYSCON_FCCLKSEL2: Register = Register::at(0x4000_02B8);
pub const SYSCON_FCCLKSEL6: Register = Register::at(0x4000_02C8);
pub const SYSCON_FCCLKSEL7: Register = Register::at(0x4000_02CC);
pub const SYSCON_FCCLKSEL_SEL: u32 = 0x7;
pub const SYSCON_FCCLKSEL_FACTORY_PLL0_DIVIDED: u32 = 1;
pub const SYSCON_AHBCLKCTRL2_SYSCTL: u32 = 1 << 15;
pub const SYSCON_PRESETCTRL2: Register = Register::at(0x4000_0108);
pub const SYSCON_PRESETCTRL2_SYSCTL_RST: u32 = 1 << 15;
pub const SYSCTL_UPDATELCKOUT: Register = Register::at(0x4002_3000);
pub const SYSCTL_UPDATELCKOUT_FACTORY_UNLOCKED: u32 = 0;
pub const SYSCTL_FCCTRLSEL: Register = Register::at(0x4002_3040);
pub const SYSCTL_FCCTRLSEL_INDEX_STEP: usize = 0x4;
pub const SYSCTL_FCCTRLSEL_FACTORY_SCK_AND_WS_FROM_SHARED_SET_0: u32 = 0x101;
pub const SYSCTL_SHAREDCTRLSET0: Register = Register::at(0x4002_3080);
pub const SYSCTL_SHAREDCTRLSET_FACTORY_SCK_AND_WS_FROM_FC7: u32 = 0x77;

// WHY: The same words on every Flexcomm but CFG1 and PSELID, SVD28 decodes them
pub const FLEXCOMM0_PSELID: Register = Register::at(0x4008_6FF8);
pub const FLEXCOMM2_PSELID: Register = Register::at(0x4008_8FF8);
pub const FLEXCOMM6_PSELID: Register = Register::at(0x4009_7FF8);
pub const FLEXCOMM7_PSELID: Register = Register::at(0x4009_8FF8);
pub const FLEXCOMM_PSELID_PERSEL: u32 = 0x7;
pub const FLEXCOMM_PSELID_I2S_TRANSMIT: u32 = 4;
pub const FLEXCOMM_PSELID_I2S_RECEIVE: u32 = 5;
pub const I2S0_CFG2: Register = Register::at(0x4008_6C04);
pub const I2S2_CFG2: Register = Register::at(0x4008_8C04);
pub const I2S6_CFG2: Register = Register::at(0x4009_7C04);
pub const I2S7_CFG2: Register = Register::at(0x4009_8C04);
pub const I2S0_DIV: Register = Register::at(0x4008_6C1C);
pub const I2S2_DIV: Register = Register::at(0x4008_8C1C);
pub const I2S6_DIV: Register = Register::at(0x4009_7C1C);
pub const I2S7_DIV: Register = Register::at(0x4009_8C1C);
pub const I2S0_FIFOCFG: Register = Register::at(0x4008_6E00);
pub const I2S2_FIFOCFG: Register = Register::at(0x4008_8E00);
pub const I2S6_FIFOCFG: Register = Register::at(0x4009_7E00);
pub const I2S7_FIFOCFG: Register = Register::at(0x4009_8E00);
pub const I2S0_FIFOWR: Register = Register::at(0x4008_6E20);
pub const I2S2_FIFOWR: Register = Register::at(0x4008_8E20);
pub const I2S6_FIFORD: Register = Register::at(0x4009_7E30);
pub const I2S7_FIFORD: Register = Register::at(0x4009_8E30);
pub const I2S_CFG1_FACTORY_MASTER_CLASSIC_32_BIT: u32 = 0x001F_0230;
pub const I2S_CFG1_FACTORY_SLAVE_CLASSIC_32_BIT: u32 = 0x001F_0200;
pub const I2S0_CFG1_MAINENABLE: u32 = 1 << 0;
pub const I2S_CFG2_FACTORY_64_BIT_FRAME: u32 = 0x3F;
pub const I2S_FIFOCFG_FACTORY_FLUSHED_AND_OFF: u32 = 0x0003_0004;
pub const I2S_FIFOCFG_FACTORY_TRANSMIT_BY_DMA_ZERO_WHEN_EMPTY: u32 = 0x1005;
pub const I2S_FIFOCFG_FACTORY_RECEIVE_BY_DMA: u32 = 0x2002;
pub const I2S0_FIFOCFG_TXI2SE0: u32 = 1 << 2;
// WHY: DIV divides the PLL0 divided clock by 8, 4 or 2 for 6, 12 or 24 frames per tick
pub const I2S_DIV_FACTORY_SINGLE_SPEED: u32 = 7;
pub const I2S_DIV_FACTORY_DOUBLE_SPEED: u32 = 3;
pub const I2S_DIV_FACTORY_QUAD_SPEED: u32 = 1;

// WHY: One self-linked reload descriptor per channel over a 96-frame buffer, no DMA interrupt, SVD28 rows and SDK fsl_dma.h descriptor layout
pub const DMA0_CTRL: Register = Register::at(0x4008_2000);
pub const DMA0_CTRL_ENABLE: u32 = 1 << 0;
pub const DMA0_SRAMBASE: Register = Register::at(0x4008_2008);
pub const DMA0_ENABLESET0: Register = Register::at(0x4008_2020);
pub const DMA0_ACTIVE0: Register = Register::at(0x4008_2030);
pub const DMA0_SETTRIG0: Register = Register::at(0x4008_2070);
pub const DMA0_CHANNEL_CFG: Register = Register::at(0x4008_2400);
pub const DMA0_CHANNEL_CFG_INDEX_STEP: usize = 0x10;
pub const DMA0_CHANNEL_CFG_PERIPHREQEN: u32 = 1 << 0;
pub const DMA0_CHANNEL_CFG_TRIGPOL: u32 = 1 << 4;
#[cfg(feature = "sound2-ui")]
pub const DMA0_CHANNEL_CFG_CHPRIORITY_LOWEST: u32 = 7 << 16;
pub const DMA0_CHANNEL_XFERCFG: Register = Register::at(0x4008_2408);
pub const DMA0_CHANNEL_XFERCFG_INDEX_STEP: usize = 0x10;
pub const DMA0_CHANNEL_XFERCFG_CFGVALID: u32 = 1 << 0;
pub const DMA0_CHANNEL_XFERCFG_RELOAD: u32 = 1 << 1;
pub const DMA0_CHANNEL_XFERCFG_SWTRIG: u32 = 1 << 2;
pub const DMA0_CHANNEL_XFERCFG_CLRTRIG: u32 = 1 << 3;
// WHY: SVD28 XFERCFG WIDTH 9:8, SRCINC 13:12, XFERCOUNT 25:16; the LCD words use WIDTH 2 and SRCINC 1
pub const DMA0_XFERCFG_WIDTH_32_BITS: u32 = 2 << 8;
pub const DMA0_XFERCFG_SOURCE_STEPS_ONE_WIDTH: u32 = 0b01 << 12;
pub const DMA0_XFERCFG_SOURCE_FIXED: u32 = 0;
pub const DMA0_XFERCFG_XFERCOUNT_SHIFT: u32 = 16;
// WHY: UM11126 Table 438, XFERCOUNT bits 25:16 count down and read 0x3FF once every transfer of the descriptor is done
pub const DMA0_XFERCFG_XFERCOUNT_EXHAUSTED: u32 = 0x3FF << 16;
pub const DMA0_FC4_TRANSMIT_CHANNEL: u32 = 1 << 13;
pub const DMA0_FC4_TRANSMIT_CHANNEL_INDEX: usize = 13;

// WHY: ADC0 setup, calibration words and the five knob commands, as in the factory configuration
pub const ADC0_CTRL: Register = Register::at(0x400A_0010);
pub const ADC0_CTRL_ADCEN: u32 = 1 << 0;
pub const ADC0_CTRL_RST: u32 = 1 << 1;
pub const ADC0_CTRL_CAL_REQ: u32 = 1 << 3;
pub const ADC0_CTRL_CALOFS: u32 = 1 << 4;
pub const ADC0_CTRL_RSTFIFO0: u32 = 1 << 8;
pub const ADC0_CTRL_RSTFIFO1: u32 = 1 << 9;
pub const ADC0_CTRL_FACTORY_SIXTEEN_CALIBRATION_AVERAGES: u32 = 4 << 16;
pub const ADC0_STAT: Register = Register::at(0x400A_0014);
pub const ADC0_STAT_RDY0: u32 = 1 << 0;
pub const ADC0_STAT_CAL_RDY: u32 = 1 << 10;
pub const ADC0_CFG: Register = Register::at(0x400A_0020);
pub const ADC0_CFG_FACTORY_KEPT: u32 = 0xEF00_FB0C;
pub const ADC0_CFG_FACTORY_SET: u32 = 0x0080_0482;
pub const ADC0_PAUSE: Register = Register::at(0x400A_0024);
pub const ADC0_PAUSE_OFF: u32 = 0;
pub const ADC0_SWTRIG: Register = Register::at(0x400A_0034);
pub const ADC0_SWTRIG_SWT0: u32 = 1 << 0;
pub const ADC0_TCTRL: Register = Register::at(0x400A_00A0);
pub const ADC0_TCTRL_INDEX_STEP: usize = 0x4;
pub const ADC0_TCTRL_FACTORY_TRIGGERS_START_COMMANDS_1_TO_4: [u32; 4] =
    [0x0100_0000, 0x0200_0000, 0x0300_0000, 0x0400_0000];
pub const ADC0_FCTRL0: Register = Register::at(0x400A_00E0);
pub const ADC0_FCTRL0_FACTORY_WATERMARK_FOUR: u32 = 0x4_0000;
pub const ADC0_FCTRL_FCOUNT_MASK: u32 = 0x1F;
pub const ADC0_GCC: Register = Register::at(0x400A_00F0);
pub const ADC0_GCC_INDEX_STEP: usize = 0x4;
pub const ADC0_GCC_RDY: u32 = 1 << 24;
// WHY: SVD28 GCC GAIN_CAL 15:0, FCTRL FCOUNT 4:0
pub const ADC0_GCC_GAIN_CAL_MASK: u32 = 0xFFFF;
pub const ADC0_GCR: Register = Register::at(0x400A_00F8);
pub const ADC0_GCR_INDEX_STEP: usize = 0x4;
pub const ADC0_CMDL1: Register = Register::at(0x400A_0100);
pub const ADC0_CMDH1: Register = Register::at(0x400A_0104);
pub const ADC0_CMDL2: Register = Register::at(0x400A_0108);
pub const ADC0_CMDH2: Register = Register::at(0x400A_010C);
pub const ADC0_CMDL3: Register = Register::at(0x400A_0110);
pub const ADC0_CMDH3: Register = Register::at(0x400A_0114);
pub const ADC0_CMDL4: Register = Register::at(0x400A_0118);
pub const ADC0_CMDH4: Register = Register::at(0x400A_011C);
pub const ADC0_CMDL5: Register = Register::at(0x400A_0120);
pub const ADC0_CMDH5: Register = Register::at(0x400A_0124);
pub const ADC0_FACTORY_KNOB_COMMANDS: [(Register, u32, Register, u32); 5] = [
    (ADC0_CMDL1, 0x80, ADC0_CMDH1, 0x0200_7500),
    (ADC0_CMDL2, 0x81, ADC0_CMDH2, 0x0300_7500),
    (ADC0_CMDL3, 0x82, ADC0_CMDH3, 0x0400_7500),
    (ADC0_CMDL4, 0x83, ADC0_CMDH4, 0x0500_7500),
    (ADC0_CMDL5, 0xA0, ADC0_CMDH5, 0x0000_7500),
];
pub const ADC0_RESFIFO0: Register = Register::at(0x400A_0300);

// WHY: CMSIS core_cm33.h SCB ICSR +0x004 with PENDSTCLR bit 25 and PENDSVCLR bit 27, VTOR +0x008
pub const SCB_ICSR: Register = Register::at(0xE000_ED04);
pub const SCB_ICSR_PENDSTCLR: u32 = 1 << 25;
pub const SCB_ICSR_PENDSVCLR: u32 = 1 << 27;
pub const SCB_VTOR: Register = Register::at(0xE000_ED08);
// WHY: CMSIS SysTick CTRL, 0 clears ENABLE and TICKINT
pub const SYST_CSR_STOPPED: u32 = 0;
// WHY: DMA0 has 23 channels, ENABLECLR0 bit n clears channel n and the bits above are reserved
pub const DMA0_EVERY_CHANNEL: u32 = (1 << 23) - 1;
pub const DMA0_CHANNEL_CTLSTAT: Register = Register::at(0x4008_2404);
pub const DMA0_CHANNEL_CTLSTAT_INDEX_STEP: usize = 0x10;
pub const DMA0_CHANNEL_CFG_FACTORY_PERIPHERAL_REQUEST: u32 = 0x11;
pub const DMA0_CHANNEL_CTLSTAT_FACTORY_CLEARED: u32 = 0;
pub const DMA0_CHANNEL_XFERCFG_FACTORY_TRANSMIT_96_FRAMES: u32 = 0x00BF_1203;
pub const DMA0_CHANNEL_XFERCFG_FACTORY_RECEIVE_96_FRAMES: u32 = 0x00BF_4203;
pub const SYSCON_AHBCLKCTRLCLR0: Register = Register::at(0x4000_0240);
// WHY: The four audio DMA channels
pub const DMA0_FC0_TRANSMIT_CHANNEL_NUMBER: usize = 5;
pub const DMA0_FC2_TRANSMIT_CHANNEL_NUMBER: usize = 11;
pub const DMA0_FC6_RECEIVE_CHANNEL_NUMBER: usize = 16;
pub const DMA0_FC7_RECEIVE_CHANNEL_NUMBER: usize = 18;

// WHY: CTIMER0 counts falling edges of the shared WS on CAP0 and interrupts every MR0 edges, SVD28 rows
pub const SYSCON_CTIMERCLKSEL0: Register = Register::at(0x4000_026C);
pub const SYSCON_CTIMERCLKSEL_SEL: u32 = 0x7;
pub const SYSCON_PRESETCTRLCLR0: Register = Register::at(0x4000_0140);
pub const SYSCON_PRESETCTRLCLR_INDEX_STEP: usize = 0x4;
pub const SYSCON_PRESETCTRL1_TIMER0_RST: u32 = 1 << 26;
pub const INPUTMUX_TIMER0CAPTSEL0: Register = Register::at(0x4000_6020);
pub const INPUTMUX_TIMER0CAPTSEL_FACTORY_SHARED_WS_0: u32 = 0x17;
pub const CTIMER0_IR: Register = Register::at(0x4000_8000);
pub const CTIMER0_TC: Register = Register::at(0x4000_8008);
pub const CTIMER0_TCR_CEN: u32 = 1 << 0;
pub const CTIMER0_TCR_CRST: u32 = 1 << 1;
pub const CTIMER0_MCR: Register = Register::at(0x4000_8014);
pub const CTIMER0_MCR_MR0I: u32 = 1 << 0;
pub const CTIMER0_MCR_MR0R: u32 = 1 << 1;
pub const CTIMER0_MCR_MR0RL: u32 = 1 << 24;
pub const CTIMER0_MR0: Register = Register::at(0x4000_8018);
pub const CTIMER0_MSR0: Register = Register::at(0x4000_8078);
pub const CTIMER0_CTCR: Register = Register::at(0x4000_8070);
pub const CTIMER0_CTCR_FACTORY_FALLING_EDGES_ON_CAP0: u32 = 2;

// WHY: CMSIS core_cm33.h IPR, four one-byte priorities per word; IRQ10 and IRQ47 run at priority 0x20, as in the factory configuration
pub const NVIC_IPR_BYTES_PER_WORD: usize = 4;
pub const NVIC_IPR_BYTE: u32 = 0xFF;
pub const NVIC_PRIORITY_FACTORY_AUDIO_LEVEL_1: u32 = 0x20;
