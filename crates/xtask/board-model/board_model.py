"""Boot the firmware image on an emulated Cortex-M33 against a model of the M2 board and print a trace.

Usage: python -I board_model.py < plan

`cargo xtask board-run` writes one plan per scenario, reads the trace and gates it. This script
judges nothing. Plan lines:
  load <elf>                     copy every PT_LOAD segment to its load (physical) address, as the flat image the loader copies does; Reset then copies .data to its run address
  entry <address>                the reset handler
  breadcrumb <address>           the image's two BREADCRUMB words, the magic and the record, watched for stores
  adc <13|4f|none> [silent-after N]  which ADC answers on bus 3, and after N transactions it stops
  dac <answers|silent|silent-after N|silent-for N>  the DAC, or `dac ramp-after N` to stop it after N volume writes
  expander <answers|silent>
  buttons <hex> <from ms> <until ms>  the expander input register reads <hex> in that window, 0xFF (none pressed) outside it
  knobs <hex> <hex> <hex> <hex> <hex>  the five RESFIFO0 knob readings, CMDSRC 1 to 5, 0x8000 each by default
  converters-need-power          the DAC answers only while P1_3 is driven high, the ADC only while P1_3 and P1_29 are
  fc4-dma <runs|stalled>         whether the FC4 transmit request reaches DMA0 channel 13, runs by default
  analog-bus <answers|never>     whether the analog word bus ever reports room for a frame
  crystal <ready|never>          whether XO32M_STATUS.XO_READY ever rises
  host <frames|none>             whether a host sends frames after DCON, none is a charger
  pll-lock <rises|never|never-at-150|after-ms N>  what PLL1STAT LOCK reads; never-at-150 reads 0 while PLL1NDEC holds 2 (150 MHz) and 1 otherwise; after-ms reads 0 until N ms after the last write to PLL1's registers or power bit
  pmc <dcdc0> <dcdc1> <ldopmu>   the PMC DCDC0, DCDC1 and LDOPMU words as the loader left them, 0 each by default
  pmc-ignores-writes             DCDC0, DCDC1 and LDOPMU keep their words whatever is written to them
  flash-word <address> <word>    the word a flash read at that address returns, 0 otherwise
  pll0-lock <rises|never|never-after N|after-ms N>  what PLL0STAT LOCK reads; never-after rises N times; after-ms reads 0 until N ms after the last write to PLL0's registers or power bits
  phy-pll-lock <rises|never>     what USBPHY PLL_SIC PLL_LOCK reads once PLL_SIC is written
  i2c-hang-from <n>              from the n-th transaction on (0 based) the master never finishes
  bus-fault-on-write <address> <nth>  a precise bus fault on the nth write (0 based) to that address
  plant-store <address> <ms>     at that time the CPU leaves the image and stores to the address
  factory-read-faults            every flash read raises a precise bus fault
  mpu-regions <n> [write-locked] MPU_TYPE.DREGION; write-locked ignores RBAR and RLAR writes
  main-clock <sela> <selb> [<ahbclkdiv> <fmccr>]  MAINCLKSELA, MAINCLKSELB, AHBCLKDIV and FMCCR as the loader left them, 0 0 0 0 by default
  ahbclkctrl <index> <word>      AHBCLKCTRL0, 1 or 2 as the loader left it; 0, 0 and USB1_DEV by default
  run <milliseconds>             boot from entry and run until the time, a reset or a stop
  probe-fence <address> <access> print what the programmed MPU decides for a read, write or fetch
Trace lines start with the emulated time in nanoseconds. ELF bytes are data. Needs unicorn 2.1.4.
"""
import heapq
import struct
import sys

from unicorn import UC_ARCH_ARM, UC_HOOK_MEM_READ, UC_HOOK_MEM_WRITE, UC_MODE_MCLASS, UC_MODE_THUMB
from unicorn import Uc, UcError
from unicorn.arm_const import UC_ARM_REG_FAULTMASK, UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_R0
from unicorn.arm_const import UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3, UC_ARM_REG_R12
from unicorn.arm_const import UC_ARM_REG_SP, UC_ARM_REG_XPSR, UC_CPU_ARM_CORTEX_M33
from unicorn.arm_const import UC_ARM_REG_FPSCR, UC_ARM_REG_PRIMASK
import unicorn.arm_const as arm_registers

BEHAVIORS = [
    ("memory-map", "DS Table 6 and 7",
     "peripherals at 0x4000_0000, flash controller block 0x4003_4000 to 0x4003_4FFF, secure alias with bit 28"),
    ("core-clock", "design choice",
     "MAINCLKSELB 2 is PLL1 at 24 MHz x M / N / 2P from PLL1MDEC, PLL1NDEC and PLL1PDEC (100 MHz until they are written, M 25 N 2 P 1 is 150 MHz); MAINCLKSELB 0 with MAINCLKSELA 0 is the 12 MHz FRO and with MAINCLKSELA 3 the 96 MHz FRO; anything else runs at 12 MHz; 12 MHz FRO at reset"),
    ("cycles-per-instruction", "guess",
     "one core cycle per executed instruction, counted in chunks of 1000 instructions"),
    ("idle-skip", "guess",
     "a SysTick COUNTFLAG poll that finds no wrap while no bus transfer is in flight jumps time to the next wrap, as if the main loop had spun until then"),
    ("systick", "CMSIS",
     "SysTick counts core cycles from RVR, COUNTFLAG sets at each wrap and clears on a CSR read"),
    ("watchdog", "windowed watchdog, DS 7.25.3.1",
     "WWDT runs at FRO 1 MHz / 4 once WDEN is set and fed, a feed is 0xAA then 0x55, a wrong feed or TC x 4 us without a feed resets"),
    ("watchdog-clock-tolerance", "DS 7.25.3.1",
     "the FRO 1 MHz is +-15%, the model runs it nominal and the gate takes the low edge"),
    ("watchdog-window", "UM11126 rev 1.8 30.6.6 and 30.6.1",
     "a feed while TV is above WINDOW resets; under WDPROTECT a TC write, or a feed while TV is above WARNINT, resets; WINDOW starts at 0xFFFFFF and WARNINT at 0"),
    ("watchdog-inherited", "guess",
     "a scenario may start with the watchdog running as a bootloader could leave it, fed at Reset with its own TC and WINDOW, with or without WDPROTECT; the factory configuration never touches the WWDT, so no bootloader is known to do this"),
    ("i2c-master", "SVD28 MSTSTATE",
     "FC3 I2C master: MSTPENDING clears on START, CONTINUE or STOP and sets when the bus step ends, MSTSTATE 0 idle, 1 receive ready, 2 transmit ready, 3 address NACK, 4 data NACK"),
    ("i2c-speed", "factory configuration",
     "SCL 400 kHz, so a byte with its ACK takes 22.5 us"),
    ("i2c-devices", "factory configuration",
     "DAC at 0x49, ADC at 0x13 or 0x4F, expander at 0x39; the first data byte of a write selects the register"),
    ("converter-power", "measured on hardware: the converters answer only once P1_3 is high",
     "under `converters-need-power` the DAC answers only while P1_3 is driven high and the ADC only while P1_3 and P1_29 are; the expander is always powered"),
    ("dac-status-byte", "guess", "the DAC answers 0x00 to a read of register 113"),
    ("expander-inputs", "guess", "the expander input register reads 0xFF, no button pressed, unless a `buttons` plan line holds a pressed byte for a window"),
    ("spi-frame", "SVD28 FIFOWR fields, factory frames",
     "FC1 FIFOWR bits 15:0 data, 16 to 19 TXSSEL0_N to TXSSEL3_N, 20 EOT, 27:24 LEN minus 1"),
    ("spi-speed", "factory configuration", "FC1 SCK 1.6 MHz from 24 MHz / 15"),
    ("spi-fifo-depth", "guess", "the FC1 TX FIFO holds 8 frames"),
    ("preamp-chain", "factory configuration",
     "the preamp chain is a 24-bit shift register, MSB first, latched when SSEL1 rises"),
    ("pll-status", "SVD28 SYSCON",
     "PLL0STAT and PLL1STAT bit 0 is LOCK; each answer is recorded once because the image polls them and must not switch a clock before the read"),
    ("usb-phy-pll-lock", "SVD28 PLL_SIC PLL_LOCK bit 31 read-only, the lock itself a guess",
     "PLL_SIC reads its last written word with PLL_LOCK set once it was written; each answer is recorded once per PLL_SIC write because the image polls it"),
    ("knob-adc", "factory configuration, the value a guess",
     "a software trigger 0 queues five RESFIFO0 words, VALID, CMDSRC 1 to 5, data 0x8000, every knob at mid travel; STAT.RDY0 and FCTRL0.FCOUNT follow the queue"),
    ("crystal-ready", "SDK fsl_clock.c via map.rs",
     "XO32M_STATUS.XO_READY reads 1 once the crystal runs; each answer is recorded once because the image polls it"),
    ("dwt-cycle-counter", "Armv8-M DWT",
     "DWT_CYCCNT reads the model's count of executed core cycles and a write to it rebases that count"),
    ("mpu", "Arm 100235",
     "Armv8-M MPU with RBAR and RLAR per region, PRIVDEFENA, HFNMIENA, overlap faults, the PPB is never checked"),
    ("mpu-regions", "guess", "MPU_TYPE.DREGION reads 8 unless the scenario says otherwise"),
    ("exceptions", "Armv8-M exception entry",
     "a fault pushes the eight-word basic frame on the main stack and enters the vector from VTOR, MemManage and BusFault escalate to HardFault unless SHCSR enables them"),
    ("bus-fault-ignored", "Armv8-M CCR.BFHFNMIGN",
     "with FAULTMASK set and CCR.BFHFNMIGN set a precise bus fault is ignored"),
    ("output-connect-inherited", "model choice",
     "P0_28 starts as an output driven high, as factory leaves it at the hand-over; only a change is traced"),
    ("usb-clock-inherited", "guess",
     "the bootloader's USB state is unmeasured, so the model starts with the USB1 device clock on, the state in which B1's conditional detach may touch the controller and in which the image is loaded over USB"),
    ("clock-gates", "SVD28 AHBCLKCTRL, AHBCLKCTRLSET and AHBCLKCTRLCLR",
     "a write to AHBCLKCTRLSETn sets and one to AHBCLKCTRLCLRn clears those bits of AHBCLKCTRLn, which reads the enable state"),
    ("core-clock-divider", "UM11126 rev 1.8 Table 102",
     "the core runs at the main clock over AHBCLKDIV DIV + 1, and a written AHBCLKDIV re-evaluates it"),
    ("unmodelled-reads", "guess",
     "a register with no model reads back the last value written, 0 at reset, as the budget bench does"),
    ("tick-interrupt", "factory configuration",
     "CTIMER0 counts WS falling edges and interrupts every MR0 edges; with CEN set, IRQ10 enabled, FC7 enabled and PRIMASK and FAULTMASK clear, IRQ10 is taken as a call of its vector at chunk boundaries"),
    ("tick-acknowledge", "SVD28 CTIMER IR, write one to clear",
     "the tick's CTIMER0.IR acknowledge is applied and not traced, so the trace does not carry 8000 lines a second"),
    ("word-select-rate", "guess", "WS runs at the 48 kHz boot rate once FC7 is enabled"),
    ("interrupt-cost", "guess", "an interrupt costs its executed instructions counted in chunks of 500, so up to 499 cycles too many"),
    ("usb-host-frames", "guess",
     "once DEVCMDSTAT.DCON is set a host is present and raises INTSTAT.FRAME_INT every 1 ms; with INTEN.FRAME_INT, IRQ47 enabled and PRIMASK and FAULTMASK clear, IRQ47 is taken as a call of its vector at chunk boundaries"),
    ("usb-frame-acknowledge", "UM11126 USB1 INTSTAT, write one to clear",
     "the USB interrupt's INTSTAT acknowledge is applied and not traced"),
    ("other-interrupts", "guess",
     "no interrupt but IRQ10 and IRQ47 is delivered, and the host sends nothing but frames: no bus reset, no setup, no stream"),
    ("flash-contents", "guess", "flash reads as zeros unless a flash-word line sets a word, so the factory read gives the template serial"),
    ("fc4-dma", "UM11126 22.5.2, 22.6.6 and Table 438",
     "DMA0 channel 13 starts once CTRL is enabled, the channel is enabled and its XFERCFG is valid and triggered by SWTRIG or SETTRIG0; it walks the head descriptor and every reload descriptor and moves 32-bit words to FC4 FIFOWR; ACTIVE0 bit 13 reads 1 until the last word enters the FIFO, then XFERCOUNT reads 0x3FF"),
    ("fc4-speed", "factory FROHFDIV 3 and SPI DIV 1 on the 96 MHz FRO",
     "FC4 SCK 12 MHz, and an EOT keeps the selects deasserted for one SCK"),
    ("fc4-fifo-depth", "UM11126 35.2", "the FC4 TX FIFO holds 8 frames; FIFOSTAT TXEMPTY reads 1 once the last frame has shifted out"),
    ("fc4-dma-stalled", "guess",
     "under `fc4-dma stalled` the FC4 request never reaches DMA0: channel 13 is triggered but never starts, ACTIVE0 reads 0 and the FIFO stays empty"),
]

RETURN_TRAP = 0x1000_0000
PLANTED_STORE = RETURN_TRAP + 0x100
PLANTED_STORE_CODE = struct.pack("<4H", 0x4801, 0x6000, 0xE7FE, 0x0000)
CHUNK = 1000
FRO_12_MHZ = 12_000_000
PLL1_100_MHZ = 100_000_000
FRO_96_MHZ = 96_000_000
WATCHDOG_TICK_NS = 4_000
I2C_BYTE_NS = 22_500
WORD_SELECT_HZ = 48_000
INTERRUPT_CHUNK = 500
TICK_IRQ = 10
USB_IRQ = 47
USB_FRAME_NS = 1_000_000
SAVED_REGISTERS = ([getattr(arm_registers, f"UC_ARM_REG_R{n}") for n in range(13)]
                   + [UC_ARM_REG_SP, UC_ARM_REG_LR, UC_ARM_REG_XPSR, UC_ARM_REG_FPSCR]
                   + [getattr(arm_registers, f"UC_ARM_REG_S{n}") for n in range(16)])
I2C_STOP_NS = 2_500
SPI_BIT_NS = 625
SPI_FIFO_DEPTH = 8
FC3_CFG = 0x4008_9800
DMA0_CTRL = 0x4008_2000
DMA0_SRAMBASE = 0x4008_2008
DMA0_ENABLESET0 = 0x4008_2020
DMA0_ENABLECLR0 = 0x4008_2028
DMA0_ACTIVE0 = 0x4008_2030
DMA0_SETTRIG0 = 0x4008_2070
FC4_CHANNEL = 13
FC4_CHANNEL_BIT = 1 << FC4_CHANNEL
DMA0_FC4_XFERCFG = 0x4008_2408 + 16 * FC4_CHANNEL
XFERCFG_CFGVALID = 1 << 0
XFERCFG_RELOAD = 1 << 1
XFERCFG_SWTRIG = 1 << 2
XFERCOUNT_EXHAUSTED = 0x3FF << 16
FC4_FIFOSTAT = 0x4008_AE04
FC4_FIFOWR = 0x4008_AE20
FIFOSTAT_TXEMPTY = 1 << 4
FIFOSTAT_TXNOTFULL = 1 << 5
FIFOWR_EOT = 1 << 20
FC4_BIT_NS = 1e9 / 12_000_000
FC4_FIFO_DEPTH = 8
FC4_MOST_DESCRIPTORS = 64

PERIPHERALS = 0x4000_0000
SECURE_PERIPHERALS = 0x5000_0000
SYSCON_AHBCLKCTRL2 = 0x4000_0208
SYSCON_AHBCLKCTRL2_USB1_DEV = 1 << 5
PERIPHERAL_SPAN = 0x10_0000
FLASH_SPAN = 0xA_0000
FLASH_CONTROLLER = (0x4003_4000, 0x4003_5000)
FACTORY_PAGE = (0x0003_8000, 0x0003_9000)
FACTORY_READ = (0x0003_8200, 0x0003_820E)
SCS = 0xE000_E000
PPB = (0xE000_0000, 0xE010_0000)
DWT_CYCCNT = 0xE000_1004
RAM = (0x2000_0000, 0x2004_0000)
RAM_ABOVE_THE_STACK = 0x2003_0000
USB_RAM = (0x4010_0000, 0x4010_4000)

BLOCKS = [
    (0x4000_0000, "SYSCON"), (0x4000_1000, "IOCON"), (0x4000_6000, "INPUTMUX"),
    (0x4000_8000, "CTIMER0"), (0x4000_C000, "WWDT"), (0x4001_3000, "ANACTRL"), (0x4002_0000, "PMC"), (0x4002_3000, "SYSCTL"),
    (0x4003_4000, "FLASHCTRL"), (0x4003_8000, "USBPHY"), (0x4008_2000, "DMA0"), (0x4008_6000, "FC0"),
    (0x4008_7000, "FC1"), (0x4008_8000, "FC2"), (0x4008_9000, "FC3"), (0x4008_A000, "FC4"),
    (0x4008_C000, "GPIO"), (0x4009_4000, "USB1"), (0x4009_7000, "FC6"), (0x4009_8000, "FC7"),
    (0x400A_0000, "ADC0"), (0x400A_3000, "USBHSH"),
]
NAMES = {
    0x4000_0100: "SYSCON.PRESETCTRL0", 0x4000_0104: "SYSCON.PRESETCTRL1",
    0x4000_0220: "SYSCON.AHBCLKCTRLSET0", 0x4000_0224: "SYSCON.AHBCLKCTRLSET1",
    0x4000_0228: "SYSCON.AHBCLKCTRLSET2", 0x4000_0280: "SYSCON.MAINCLKSELA",
    0x4000_0284: "SYSCON.MAINCLKSELB", 0x4000_0290: "SYSCON.PLL0CLKSEL", 0x4000_0294: "SYSCON.PLL1CLKSEL",
    0x4000_02B4: "SYSCON.FCCLKSEL1", 0x4000_02BC: "SYSCON.FCCLKSEL3", 0x4000_02C0: "SYSCON.FCCLKSEL4",
    0x4000_02E0: "SYSCON.MCLKCLKSEL",
    0x4000_0380: "SYSCON.AHBCLKDIV", 0x4000_0400: "SYSCON.FMCCR", 0x4000_0388: "SYSCON.FROHFDIV", 0x4000_038C: "SYSCON.WDTCLKDIV",
    0x4000_03AC: "SYSCON.MCLKDIV", 0x4000_03C4: "SYSCON.PLL0CLKDIV", 0x4000_0420: "SYSCON.MCLKIO",
    0x4000_0560: "SYSCON.PLL1CTRL", 0x4000_0564: "SYSCON.PLL1STAT", 0x4000_0568: "SYSCON.PLL1NDEC",
    0x4000_056C: "SYSCON.PLL1MDEC", 0x4000_0570: "SYSCON.PLL1PDEC", 0x4000_0580: "SYSCON.PLL0CTRL",
    0x4000_0584: "SYSCON.PLL0STAT", 0x4000_0588: "SYSCON.PLL0NDEC", 0x4000_058C: "SYSCON.PLL0PDEC",
    0x4000_0594: "SYSCON.PLL0SSCG1", 0x4000_0A18: "SYSCON.CLOCK_CTRL", 0x4000_8004: "CTIMER0.TCR",
    0x4000_C000: "WWDT.MOD", 0x4000_C004: "WWDT.TC", 0x4000_C014: "WWDT.WARNINT",
    0x4000_C018: "WWDT.WINDOW", 0x4001_3010: "ANACTRL.FRO192M_CTRL", 0x4001_3020: "ANACTRL.XO32M_CTRL",
    0x4001_3024: "ANACTRL.XO32M_STATUS", 0x4002_0098: "PMC.RTCOSC32K",
    0x4002_00C8: "PMC.PDRUNCFGCLR0", 0x4002_00C0: "PMC.PDRUNCFGSET0", 0x4002_0010: "PMC.DCDC0",
    0x4002_0014: "PMC.DCDC1", 0x4002_001C: "PMC.LDOPMU", 0x4003_8034: "USBPHY.CTRL_SET", 0x4003_80A0: "USBPHY.PLL_SIC", 0x4008_2000: "DMA0.CTRL", 0x4008_2020: "DMA0.ENABLESET0",
    0x4008_2028: "DMA0.ENABLECLR0", 0x4008_6C00: "FC0.CFG1", 0x4008_8C00: "FC2.CFG1",
    0x4009_7C00: "FC6.CFG1", 0x4009_8C00: "FC7.CFG1", 0x4008_7FF8: "FC1.PSELID", 0x4008_7400: "FC1.CFG",
    0x4008_7424: "FC1.DIV", 0x4008_7E00: "FC1.FIFOCFG", 0x4008_9FF8: "FC3.PSELID", 0x4008_9800: "FC3.CFG",
    0x4008_9814: "FC3.CLKDIV", 0x4008_9824: "FC3.MSTTIME", 0x4009_4000: "USB1.DEVCMDSTAT",
    0x4009_4008: "USB1.EPLISTSTART",
}
P0_28 = 1 << 28
PIN_PAD = {0x4000_1000 + 4 * (32 * port + pin): f"IOCON.P{port}_{pin}" for port in (0, 1) for pin in range(32)}
NAMES.update(PIN_PAD)

def block_of(address):
    name = "PERIPHERAL"
    for base, block in BLOCKS:
        if address >= base:
            name = block
    return name


def name_of(address):
    if address in NAMES:
        return NAMES[address]
    for base, block in reversed(BLOCKS):
        if address >= base:
            return f"{block}+{address - base:#x}"
    return f"{address:#x}"


def load(uc, path):
    data = open(path, "rb").read()
    phoff, = struct.unpack_from("<I", data, 0x1C)
    phentsize, phnum = struct.unpack_from("<HH", data, 0x2A)
    for i in range(phnum):
        kind, offset, _, paddr, filesz, _, _, _ = struct.unpack_from("<8I", data, phoff + i * phentsize)
        if kind == 1 and filesz:
            uc.mem_write(paddr, data[offset:offset + filesz])


KNOB_READING = 0x8000


class Ended(Exception):
    pass


class Board:
    def __init__(self, out):
        self.out = out
        self.cycles = 0
        self.now_ns = 0.0
        self.core_hz = FRO_12_MHZ
        self.pll_configured_ns = [0, 0]
        self.pending = []
        self.sequence = 0
        self.registers = {SYSCON_AHBCLKCTRL2: SYSCON_AHBCLKCTRL2_USB1_DEV}
        self.ended = None
        self.exception = None
        self.uc = None
        self.config = {"adc": 0x13, "adc-mode": "answers", "adc-after": 0, "dac": ("answers", 0), "expander": "answers", "crystal": "ready", "host": "frames",
                       "pll-lock": "rises", "pll0-lock": "rises", "phy-pll-lock": "rises", "pll0-lock-after": 0, "i2c-hang-from": None, "bus-fault": None, "mpu-regions": 8,
                       "analog-bus": "answers", "dac-ramp-after": None, "fault-code": None, "factory-read-faults": False, "flash-words": {}, "pmc-ignores-writes": False, "pll-lock-ms": 0, "pll0-lock-ms": 0,
                       "plant-store": None, "handover-writes": [], "buttons": [],
                       "knobs": [KNOB_READING] * 5, "converters-need-power": False}
        self.pll0_reads = 0
        self.writes_seen = {}
        self.pll_status_reported = {}
        self.dwt_epoch = 0
        self.dwt_published = 0
        self.systick = {"csr": 0, "rvr": 0, "epoch": 0, "read_wraps": 0}
        self.watchdog = {"mod": 0, "tc": 0xFF, "window": 0xFF_FFFF, "warnint": 0, "running": False, "deadline": None, "feed": None}
        self.mpu = {"ctrl": 0, "rnr": 0, "rbar": {}, "rlar": {}}
        self.scb = {0xE000_ED08: 0, 0xE000_ED14: 0x201, 0xE000_ED24: 0, 0xE000_ED28: 0}
        self.gpio = {"level": {0: P0_28, 1: 0}, "dir": {0: P0_28, 1: 0}, "inputs": {0: 0, 1: 0}}
        self.spi = {"frames": [], "busy_until": 0.0, "chain": 0, "selected": False, "shifted": 0}
        self.fc4 = {"mode": "runs", "armed": False, "triggered": False, "active": False, "dma_done": 0.0,
                    "frame_ends": [], "spi_done": 0.0, "transfers": [], "errors": []}
        self.i2c = I2cMaster(self)
        self.tick_due = None
        self.frame_due = None
        self.knob_fifo = []
        self.delivered = {}

    def emit(self, kind, at=None):
        when = self.now_ns if at is None else at
        self.sequence += 1
        heapq.heappush(self.pending, (when, self.sequence, kind))
        self.flush(self.now_ns)

    def flush(self, until):
        while self.pending and self.pending[0][0] <= until:
            when, _, kind = heapq.heappop(self.pending)
            self.out.write(f"{int(when)} {kind}\n")

    def end(self, reason, at=None):
        if self.ended is None:
            if at is not None:
                self.now_ns = max(self.now_ns, at)
            self.ended = reason
            self.emit(f"end {reason}")
            self.uc.emu_stop()

    def advance_cycles(self, cycles):
        self.cycles += cycles
        self.now_ns += cycles * 1e9 / self.core_hz
        self.publish_cycle_counter()
        self.check_watchdog()

    def publish_cycle_counter(self):
        written, = struct.unpack("<I", self.uc.mem_read(DWT_CYCCNT, 4))
        if written != self.dwt_published:
            self.dwt_epoch = (self.cycles - written) & 0xFFFF_FFFF
        self.dwt_published = (self.cycles - self.dwt_epoch) & 0xFFFF_FFFF
        self.uc.mem_write(DWT_CYCCNT, struct.pack("<I", self.dwt_published))

    def check_watchdog(self):
        deadline = self.watchdog["deadline"]
        if deadline is not None and self.now_ns >= deadline:
            self.now_ns = deadline
            self.emit("watchdog starved")
            self.end("watchdog-reset", deadline)

    def in_flight(self):
        return self.i2c.busy() or self.now_ns < self.spi["busy_until"]

    def tick_period_ns(self):
        running = (self.stored(0x4000_8004) & 1 and self.stored(0xE000_E100) & (1 << 10)
                   and self.stored(0x4009_8C00) & 1)
        if not running:
            return None
        return max(1, self.stored(0x4000_8018)) * 1e9 / WORD_SELECT_HZ

    def tick_pending(self):
        period = self.tick_period_ns()
        if period is None:
            self.tick_due = None
            return False
        if self.tick_due is None:
            self.tick_due = self.now_ns + period
        if self.now_ns < self.tick_due or self.interrupts_masked():
            return False
        self.tick_due = max(self.tick_due + period, self.now_ns)
        return True

    def usb_frame_pending(self):
        running = (self.stored(0x4009_4000) & (1 << 16) and self.stored(0x4009_4024) & (1 << 30)
                   and self.stored(0xE000_E104) & (1 << (USB_IRQ - 32)))
        if not running or self.config["host"] == "none":
            self.frame_due = None
            return False
        if self.frame_due is None:
            self.frame_due = self.now_ns + USB_FRAME_NS
        if self.now_ns < self.frame_due or self.interrupts_masked():
            return False
        self.frame_due = max(self.frame_due + USB_FRAME_NS, self.now_ns)
        self.store(0x4009_4020, self.stored(0x4009_4020) | (1 << 30))
        return True

    def faultmask(self):
        return self.uc.reg_read(UC_ARM_REG_FAULTMASK) & 1

    def interrupts_masked(self):
        return self.uc.reg_read(UC_ARM_REG_PRIMASK) & 1 or self.faultmask()

    def mpu_decides(self, address, access, ignoring_priority=False):
        ctrl = self.mpu["ctrl"]
        if not ctrl & 1 or PPB[0] <= address < PPB[1]:
            return "allowed"
        if not ignoring_priority and self.faultmask() and not ctrl & 2:
            return "allowed"
        hits = []
        for number in range(self.config["mpu-regions"]):
            rlar = self.mpu["rlar"].get(number, 0)
            rbar = self.mpu["rbar"].get(number, 0)
            if rlar & 1 and (rbar & ~0x1F) <= address <= ((rlar & ~0x1F) | 0x1F):
                hits.append(rbar)
        if not hits:
            return "allowed" if ctrl & 4 else "fault no-region"
        if len(hits) > 1:
            return "fault overlap"
        rbar = hits[0]
        if access == "write" and (rbar >> 1) & 2:
            return "fault read-only"
        if access == "fetch" and rbar & 1:
            return "fault execute-never"
        return "allowed"

    def guard(self, address, access):
        verdict = self.mpu_decides(address, access)
        if verdict == "allowed":
            return True
        self.emit(f"mpu-fault {address:#x} {access} {verdict.split(' ', 1)[1]}")
        self.raise_fault(4 if self.scb[0xE000_ED24] & (1 << 16) else 3)
        return False

    def raise_fault(self, number):
        if self.exception is None:
            self.exception = number
            self.uc.emu_stop()

    def take_exception(self):
        uc = self.uc
        number, self.exception = self.exception, None
        pc = uc.reg_read(UC_ARM_REG_PC)
        sp = (uc.reg_read(UC_ARM_REG_SP) - 32) & ~7
        frame = [uc.reg_read(r) for r in (UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3,
                                          UC_ARM_REG_R12, UC_ARM_REG_LR)]
        frame += [pc, uc.reg_read(UC_ARM_REG_XPSR) | (1 << 24)]
        uc.mem_write(sp, struct.pack("<8I", *frame))
        handler, = struct.unpack("<I", uc.mem_read(self.scb[0xE000_ED08] + 4 * number, 4))
        uc.reg_write(UC_ARM_REG_SP, sp)
        uc.reg_write(UC_ARM_REG_LR, 0xFFFF_FFF9)
        uc.reg_write(UC_ARM_REG_XPSR, (frame[7] & ~0x1FF) | number)
        self.emit(f"exception {number} from {pc:#x}")
        return handler

    def store(self, address, value):
        self.registers[address & ~3] = value & 0xFFFF_FFFF

    def stored(self, address):
        return self.registers.get(address & ~3, 0)

    def converter_powered(self, device):
        if not self.config["converters-need-power"] or device.name == "expander":
            return True
        converter = self.pin_level(1, 3)
        if device.name == "dac":
            return bool(converter)
        return bool(converter and self.pin_level(1, 29))

    def expander_inputs(self):
        for start, until, value in self.config["buttons"]:
            if start <= self.now_ns < until:
                return value
        return 0xFF

    def read_peripheral(self, address):
        if address in (0x4000_0564, 0x4000_0584):
            if address == 0x4000_0584 and self.config["pll0-lock"] == "never-after":
                rising = self.pll0_reads < self.config["pll0-lock-after"]
                self.pll0_reads += 1
                value = 1 if rising else 0
            else:
                key = "pll-lock" if address == 0x4000_0564 else "pll0-lock"
                value = 1 if self.config[key] == "rises" else 0
                if self.config[key] == "never-at-150":
                    value = 0 if self.stored(0x4000_0568) == 2 else 1
                elif self.config[key] == "after-ms":
                    which = 1 if address == 0x4000_0564 else 0
                    settled = self.pll_configured_ns[which] + self.config[key + "-ms"] * 1_000_000
                    value = 1 if self.now_ns >= settled else 0
            if self.pll_status_reported.get(address) != value:
                self.pll_status_reported[address] = value
                self.emit(f"read {name_of(address)}")
            return value
        if address == 0x4003_80A0:
            locked = self.config["phy-pll-lock"] == "rises" and address in self.registers
            if self.pll_status_reported.get(address) != locked:
                self.pll_status_reported[address] = locked
                self.emit(f"read {name_of(address)}")
            return self.stored(address) | (1 << 31 if locked else 0)
        if address == 0x4001_3024:
            ready = self.config["crystal"] == "ready" and self.stored(0x4001_3020) & (1 << 24)
            value = 1 if ready else 0
            if self.pll_status_reported.get(address) != value:
                self.pll_status_reported[address] = value
                self.emit(f"read {name_of(address)}")
            return value
        if 0x4008_7000 <= address < 0x4008_8000:
            return self.read_spi(address)
        if 0x4008_9000 <= address < 0x4008_A000:
            return self.i2c.read(address)
        if 0x4008_C000 <= address < 0x4009_0000:
            return self.read_gpio(address)
        if address == 0x4000_C00C:
            return self.watchdog_count()
        if address == 0x400A_0014:
            return (1 << 10) | (1 if self.knob_fifo else 0)
        if address == 0x400A_00E0:
            return (self.stored(address) & ~0x1F) | len(self.knob_fifo)
        if address == 0x400A_0300:
            return self.knob_fifo.pop(0) if self.knob_fifo else 0
        if 0x400A_00F0 <= address < 0x400A_00F8:
            return (1 << 24) | self.stored(address)
        if address == FC4_FIFOSTAT:
            pending = sum(1 for end in self.fc4["frame_ends"] if end > self.now_ns)
            return ((FIFOSTAT_TXEMPTY if pending == 0 else 0)
                    | (FIFOSTAT_TXNOTFULL if pending < FC4_FIFO_DEPTH else 0))
        if address == DMA0_ACTIVE0:
            self.settle_fc4()
            return FC4_CHANNEL_BIT if self.fc4["active"] else 0
        if address == DMA0_FC4_XFERCFG:
            self.settle_fc4()
        return self.stored(address)

    def settle_fc4(self):
        fc4 = self.fc4
        if fc4["active"] and self.now_ns >= fc4["dma_done"]:
            fc4["active"] = False
            self.store(DMA0_FC4_XFERCFG, self.stored(DMA0_FC4_XFERCFG) | XFERCOUNT_EXHAUSTED)

    def program_fc4(self, address, value):
        fc4 = self.fc4
        if address == DMA0_FC4_XFERCFG:
            fc4["armed"] = bool(value & XFERCFG_CFGVALID)
            fc4["triggered"] = bool(value & XFERCFG_SWTRIG)
        elif address == DMA0_SETTRIG0 and value & FC4_CHANNEL_BIT:
            fc4["triggered"] = True
        elif address == DMA0_ENABLECLR0 and value & FC4_CHANNEL_BIT:
            self.store(DMA0_ENABLESET0, self.stored(DMA0_ENABLESET0) & ~FC4_CHANNEL_BIT)
        enabled = self.stored(DMA0_CTRL) & 1 and self.stored(DMA0_ENABLESET0) & FC4_CHANNEL_BIT
        if fc4["armed"] and fc4["triggered"] and enabled and fc4["mode"] == "runs":
            fc4["armed"] = fc4["triggered"] = False
            self.start_fc4()

    def start_fc4(self):
        fc4 = self.fc4
        uc = self.uc
        config = self.stored(DMA0_FC4_XFERCFG)
        head = self.stored(DMA0_SRAMBASE) + 16 * FC4_CHANNEL
        source_end, destination_end, link = struct.unpack("<3I", uc.mem_read(head + 4, 12))
        words = []
        for _ in range(FC4_MOST_DESCRIPTORS):
            count = ((config >> 16) & 0x3FF) + 1
            width = 1 << ((config >> 8) & 3)
            increment = (config >> 12) & 3
            step = 0 if increment == 0 else width << (increment - 1)
            if width != 4 or destination_end != FC4_FIFOWR or (config >> 14) & 3:
                fc4["errors"].append(f"descriptor {config:#x} {source_end:#x} {destination_end:#x}")
                break
            first = source_end - (count - 1) * step
            if step == 0:
                words += [struct.unpack("<I", uc.mem_read(first, 4))[0]] * count
            else:
                words += list(struct.unpack(f"<{count}I", uc.mem_read(first, 4 * count)))
            if not config & XFERCFG_RELOAD:
                break
            config, source_end, destination_end, link = struct.unpack("<4I", uc.mem_read(link, 16))
            self.store(DMA0_FC4_XFERCFG, config)
        at = max(self.now_ns, fc4["spi_done"])
        ends = [end for end in fc4["frame_ends"] if end > self.now_ns]
        for word in words:
            at += (((word >> 24) & 0xF) + 1 + (1 if word & FIFOWR_EOT else 0)) * FC4_BIT_NS
            ends.append(at)
        fc4["frame_ends"] = ends
        fc4["spi_done"] = at
        fc4["dma_done"] = ends[-FC4_FIFO_DEPTH - 1] if len(ends) > FC4_FIFO_DEPTH else self.now_ns
        fc4["active"] = True
        fc4["transfers"].append((self.now_ns, words))

    def write_peripheral(self, address, value):
        if address == 0x4003_80A0:
            self.pll_status_reported.pop(address, None)
        if 0x4008_7000 <= address < 0x4008_8000:
            self.write_spi(address, value)
            return
        if 0x4008_9000 <= address < 0x4008_A000:
            self.i2c.write(address, value)
            return
        if 0x4008_C000 <= address < 0x4009_0000:
            self.write_gpio(address, value)
            return
        if 0x4000_C000 <= address < 0x4000_D000:
            self.write_watchdog(address, value)
            return
        if address == 0x4009_4000:
            before = self.stored(address) & (1 << 16)
            kept = value & ~((1 << 8) | (1 << 24) | (1 << 25) | (1 << 26))
            self.store(address, kept)
            self.emit(f"write {name_of(address)} {value:#x}")
            after = kept & (1 << 16)
            if after != before:
                self.emit("usb connect" if after else "usb disconnect")
            return
        if 0x4000_0560 <= address <= 0x4000_0570 or (address in (0x4002_00C0, 0x4002_00C8) and value & (1 << 10)):
            self.pll_configured_ns[1] = self.now_ns
        if 0x4000_0580 <= address <= 0x4000_05A0 or (address in (0x4002_00C0, 0x4002_00C8) and value & ((1 << 9) | (1 << 23))):
            self.pll_configured_ns[0] = self.now_ns
        if address == 0x4000_8000:
            self.store(address, 0)
            return
        if address in (0x4002_0010, 0x4002_0014, 0x4002_001C) and self.config["pmc-ignores-writes"]:
            self.emit(f"write {name_of(address)} {value:#x} ignored")
            return
        if address == 0x4009_4020:
            self.store(address, self.stored(address) & ~value)
            return
        if address == 0x400A_0034 and value & 1:
            self.knob_fifo = [(1 << 31) | (source << 24) | reading
                              for source, reading in enumerate(self.config["knobs"], start=1)]
        if address == 0x4000_0104:
            self.reset_flexcomms(self.stored(address), value)
        if 0x4000_0220 <= address < 0x4000_022C:
            gate = 0x4000_0200 + address - 0x4000_0220
            self.store(gate, self.stored(gate) | value)
        if 0x4000_0240 <= address < 0x4000_024C:
            gate = 0x4000_0200 + address - 0x4000_0240
            self.store(gate, self.stored(gate) & ~value)
        self.store(address, value)
        self.emit(f"write {name_of(address)} {value:#x}")
        if address in (DMA0_FC4_XFERCFG, DMA0_SETTRIG0, DMA0_ENABLESET0, DMA0_ENABLECLR0, DMA0_CTRL):
            self.program_fc4(address, value)
        if address in (0x4000_0280, 0x4000_0284, 0x4000_0380):
            self.select_main_clock(always_report=address == 0x4000_0284)

    def select_main_clock(self, always_report):
        before = self.core_hz
        a, b = self.stored(0x4000_0280) & 7, self.stored(0x4000_0284) & 7
        divisor = (self.stored(0x4000_0380) & 0xFF) + 1
        if b == 2:
            m, n, p = self.stored(0x4000_056C), self.stored(0x4000_0568), self.stored(0x4000_0570)
            pll1 = 24_000_000 * m // n // (2 * p) if m and n and p else PLL1_100_MHZ
            self.core_hz = pll1 // divisor
        elif b == 0 and a == 3:
            self.core_hz = FRO_96_MHZ // divisor
        else:
            self.core_hz = FRO_12_MHZ // divisor
        if always_report or self.core_hz != before:
            self.emit(f"clock main {self.core_hz}")

    def reset_flexcomms(self, before, after):
        if after & (1 << 12) and not before & (1 << 12):
            self.flush_spi("reset")
        if after & (1 << 14) and not before & (1 << 14):
            self.i2c.reset("reset")

    def watchdog_count(self):
        deadline = self.watchdog["deadline"]
        return 0 if deadline is None else max(0, int((deadline - self.now_ns) / WATCHDOG_TICK_NS))

    def inherit_watchdog(self, mode, tc, window):
        dog = self.watchdog
        dog.update(mod=mode, tc=tc, window=window, running=True, deadline=self.now_ns + tc * WATCHDOG_TICK_NS)
        self.store(0x4000_C000, mode)
        self.store(0x4000_C004, tc)
        self.store(0x4000_C018, window)

    def feed_refusal(self):
        dog = self.watchdog
        count = self.watchdog_count()
        if count > dog["window"]:
            return f"tv={count:#x} above window={dog['window']:#x}"
        if dog["mod"] & 0x10 and count > dog["warnint"]:
            return f"tv={count:#x} above warnint={dog['warnint']:#x} under WDPROTECT"
        return None

    def write_watchdog(self, address, value):
        dog = self.watchdog
        if address == 0x4000_C008:
            if value == 0xAA:
                dog["feed"] = "first"
                return
            if value == 0x55 and dog["feed"] == "first":
                dog["feed"] = None
                if dog["mod"] & 1:
                    refusal = self.feed_refusal() if dog["running"] else None
                    if refusal is not None:
                        self.emit(f"watchdog bad-feed {refusal}")
                        self.end("watchdog-reset")
                        return
                    if not dog["running"]:
                        dog["running"] = True
                        self.emit(f"watchdog arm tc={dog['tc']} mod={dog['mod']:#x}")
                    else:
                        self.emit("watchdog feed")
                    dog["deadline"] = self.now_ns + dog["tc"] * WATCHDOG_TICK_NS
                return
            self.emit(f"watchdog bad-feed {value:#x}")
            if dog["running"]:
                self.end("watchdog-reset")
            return
        if dog["feed"] == "first" and dog["running"]:
            self.emit(f"watchdog bad-feed write to {name_of(address)}")
            self.end("watchdog-reset")
            return
        dog["feed"] = None
        self.emit(f"write {name_of(address)} {value:#x}")
        if address == 0x4000_C000:
            dog["mod"] |= value & 0x13
            dog["mod"] = (dog["mod"] & ~0xC) | (value & 0xC)
        elif address == 0x4000_C004:
            if dog["running"] and dog["mod"] & 0x10:
                self.emit("watchdog bad-feed TC write under WDPROTECT")
                self.end("watchdog-reset")
                return
            dog["tc"] = max(0xFF, value & 0xFF_FFFF)
        elif address == 0x4000_C018:
            dog["window"] = value & 0xFF_FFFF
        elif address == 0x4000_C014:
            dog["warnint"] = value & 0x3FF
        self.store(address, value)

    def read_gpio(self, address):
        g = self.gpio
        if 0x4008_C000 <= address < 0x4008_C040:
            port, pin = divmod(address - 0x4008_C000, 0x20)
            return (self.pin_level(port, pin)) & 1
        if 0x4008_D000 <= address < 0x4008_D100:
            port, pin = divmod((address - 0x4008_D000) // 4, 32)
            return 0xFFFF_FFFF if self.pin_level(port, pin) else 0
        port = (address & 0x7F) // 4
        if port not in (0, 1):
            return 0
        base = address & ~0x7F
        if base == 0x4008_E000:
            return g["dir"][port]
        if base == 0x4008_E100:
            return (g["level"][port] & g["dir"][port]) | (g["inputs"][port] & ~g["dir"][port])
        if base == 0x4008_E200:
            return g["level"][port]
        return 0

    def pin_level(self, port, pin):
        g = self.gpio
        if g["dir"][port] >> pin & 1:
            return g["level"][port] >> pin & 1
        return g["inputs"][port] >> pin & 1

    def write_gpio(self, address, value):
        g = self.gpio
        before = {port: (g["level"][port], g["dir"][port]) for port in (0, 1)}
        if 0x4008_C000 <= address < 0x4008_C040:
            port, pin = divmod(address - 0x4008_C000, 0x20)
            self.set_level(port, 1 << pin, value & 1)
        elif 0x4008_D000 <= address < 0x4008_D100:
            port, pin = divmod((address - 0x4008_D000) // 4, 32)
            self.set_level(port, 1 << pin, 1 if value else 0)
        else:
            port = (address & 0x7F) // 4
            base = address & ~0x7F
            if port not in (0, 1):
                self.emit(f"write {name_of(address)} {value:#x}")
                return
            if base == 0x4008_E380:
                g["dir"][port] |= value
            elif base == 0x4008_E400:
                g["dir"][port] &= ~value
            elif base == 0x4008_E000:
                g["dir"][port] = value
            elif base == 0x4008_E200:
                g["level"][port] |= value
            elif base == 0x4008_E280:
                g["level"][port] &= ~value
            elif base == 0x4008_E300:
                g["level"][port] ^= value
            else:
                self.emit(f"write {name_of(address)} {value:#x}")
                return
        for port in (0, 1):
            old_level, old_dir = before[port]
            for pin in range(32):
                bit = 1 << pin
                was = (old_dir & bit, old_level & bit)
                now = (g["dir"][port] & bit, g["level"][port] & bit)
                if was != now and now[0]:
                    self.emit(f"pin P{port}_{pin} {'high' if now[1] else 'low'}")
                elif was != now and was[0] and not now[0]:
                    self.emit(f"pin P{port}_{pin} released")

    def set_level(self, port, bit, high):
        if high:
            self.gpio["level"][port] |= bit
        else:
            self.gpio["level"][port] &= ~bit

    def read_spi(self, address):
        if address == 0x4008_7E04:
            unsent = [f for f in self.spi["frames"] if f[0] > self.now_ns]
            status = 0
            if not unsent:
                status |= 1 << 4
            if self.config["analog-bus"] != "never" and len([f for f in self.spi["frames"] if f[1] > self.now_ns]) < SPI_FIFO_DEPTH:
                status |= 1 << 5
            return status
        if address == 0x4008_7408:
            return (1 << 8) if self.now_ns >= self.spi["busy_until"] else 0
        return self.stored(address)

    def write_spi(self, address, value):
        if address == 0x4008_7E20:
            self.push_frame(value)
            return
        if address == 0x4008_7E00 and value & (1 << 16):
            self.flush_spi("emptytx")
        self.store(address, value)
        self.emit(f"write {name_of(address)} {value:#x}")

    def push_frame(self, value):
        spi = self.spi
        bits = ((value >> 24) & 0xF) + 1
        start = max(self.now_ns, spi["busy_until"])
        end = start + bits * SPI_BIT_NS
        spi["busy_until"] = end
        spi["frames"] = [f for f in spi["frames"] if f[1] > self.now_ns] + [(start, end)]
        selected = not (value >> 17) & 1
        if spi["selected"] and not selected:
            self.latch(start)
        if selected and not spi["selected"]:
            spi["shifted"] = 0
        spi["selected"] = selected
        data = value & ((1 << bits) - 1)
        spi["chain"] = ((spi["chain"] << bits) | data) & 0xFF_FFFF
        if selected:
            spi["shifted"] += bits
        if selected and (value >> 20) & 1:
            self.latch(end)
            spi["selected"] = False

    def latch(self, at):
        spi = self.spi
        self.emit(f"preamp-word bits={spi['shifted']} value={spi['chain']:#08x}", at)
        spi["shifted"] = 0

    def flush_spi(self, why):
        spi = self.spi
        cut = [f for f in spi["frames"] if f[0] < self.now_ns < f[1]]
        if spi["selected"] or cut:
            self.emit(f"spi-cut {why}")
        if spi["selected"]:
            self.latch(self.now_ns)
            spi["selected"] = False
        spi["frames"] = [f for f in spi["frames"] if f[1] <= self.now_ns]
        spi["busy_until"] = min(spi["busy_until"], self.now_ns)

    def read_scs(self, address):
        if address == 0xE000_E010:
            return self.read_systick_control()
        if address == 0xE000_E014:
            return self.systick["rvr"]
        if address == 0xE000_E018:
            period = self.systick["rvr"] + 1
            return self.systick["rvr"] - (self.cycles - self.systick["epoch"]) % period
        if address == 0xE000_ED90:
            return self.config["mpu-regions"] << 8
        if address == 0xE000_ED94:
            return self.mpu["ctrl"]
        if address == 0xE000_ED98:
            return self.mpu["rnr"]
        if address == 0xE000_ED9C:
            return self.mpu["rbar"].get(self.mpu["rnr"], 0)
        if address == 0xE000_EDA0:
            return self.mpu["rlar"].get(self.mpu["rnr"], 0)
        if address == 0xE000_ED0C:
            return 0xFA05_0000 | (self.scb.get(address, 0) & 0x700)
        if address in self.scb:
            return self.scb[address]
        return self.stored(address)

    def read_systick_control(self):
        st = self.systick
        csr = st["csr"] & 0x7
        if not csr & 1:
            return csr
        period = st["rvr"] + 1
        wraps = (self.cycles - st["epoch"]) // period
        if wraps > st["read_wraps"]:
            st["read_wraps"] = wraps
            return csr | (1 << 16)
        if self.in_flight():
            return csr
        target = st["epoch"] + (wraps + 1) * period
        if self.tick_due is not None and self.tick_period_ns() is not None and not self.interrupts_masked():
            tick_cycles = self.cycles + int((self.tick_due - self.now_ns) * self.core_hz / 1e9) + 1
            if tick_cycles < target:
                if tick_cycles > self.cycles:
                    self.advance_cycles(tick_cycles - self.cycles)
                return csr
        self.advance_cycles(target - self.cycles)
        st["read_wraps"] = wraps + 1
        return csr | (1 << 16)

    def write_scs(self, address, value):
        if address == 0xE000_E010:
            self.systick["csr"] = value
        elif address == 0xE000_E014:
            self.systick["rvr"] = value & 0xFF_FFFF
        elif address == 0xE000_E018:
            self.systick["epoch"] = self.cycles
            self.systick["read_wraps"] = 0
        elif address == 0xE000_ED94:
            self.mpu["ctrl"] = value & 7
            if value & 1:
                regions = " ".join(f"{self.mpu['rbar'].get(n, 0):#x}-{self.mpu['rlar'].get(n, 0):#x}"
                                   for n in range(self.config["mpu-regions"])
                                   if self.mpu["rlar"].get(n, 0) & 1)
                self.emit(f"mpu enable ctrl={value & 7:#x} regions {regions}")
            else:
                self.emit("mpu disable")
        elif address == 0xE000_ED98:
            self.mpu["rnr"] = value & 0xFF
        elif address in (0xE000_ED9C, 0xE000_EDA0) and self.config.get("mpu-write-locked"):
            pass
        elif address == 0xE000_ED9C:
            self.mpu["rbar"][self.mpu["rnr"]] = value
        elif address == 0xE000_EDA0:
            self.mpu["rlar"][self.mpu["rnr"]] = value
        elif address == 0xE000_ED0C:
            self.scb[address] = value
            if value >> 16 == 0x05FA and value & 4:
                self.emit("reset-request")
                self.end("reset-request")
        elif address == 0xE000_ED28:
            self.scb[address] &= ~value
        elif address in self.scb:
            self.scb[address] = value
        elif 0xE000_E100 <= address < 0xE000_E500:
            self.store(address, value)
            self.emit(f"write NVIC+{address - 0xE000_E100:#x} {value:#x}")
        else:
            self.store(address, value)

    def bus_fault_on_read(self, address):
        self.scb[0xE000_ED28] |= (1 << 9) | (1 << 15)
        if self.faultmask() and self.scb[0xE000_ED14] & (1 << 8):
            self.emit(f"bus-fault ignored {address:#x}")
            return
        self.emit(f"bus-fault {address:#x}")
        self.raise_fault(5 if self.scb[0xE000_ED24] & (1 << 17) else 3)

    def bus_fault_due(self, address):
        fault = self.config["bus-fault"]
        if fault is None or fault[0] != address:
            return False
        seen = self.writes_seen.get(address, 0)
        self.writes_seen[address] = seen + 1
        if seen != fault[1]:
            return False
        if self.faultmask() and self.scb[0xE000_ED14] & (1 << 8):
            self.emit(f"bus-fault ignored {address:#x}")
            return True
        self.emit(f"bus-fault {address:#x}")
        self.scb[0xE000_ED28] |= (1 << 9) | (1 << 15)
        self.raise_fault(5 if self.scb[0xE000_ED24] & (1 << 17) else 3)
        return True


class Device:
    def __init__(self, name, address, mode, after=0, power_on=None, ramp_after=None):
        self.name = name
        self.address = address
        self.mode = mode
        self.after = after
        self.transactions = 0
        self.ramp_writes = 0
        self.ramp_after = ramp_after
        self.pointer = 0
        self.registers = dict(power_on or {})

    def answers(self):
        if self.mode == "answers":
            return True
        if self.mode == "silent-after":
            return self.transactions < self.after
        if self.mode == "silent-for":
            return self.transactions >= self.after
        return False


class I2cMaster:
    def __init__(self, board):
        self.board = board
        self.devices = {}
        self.transaction_count = 0
        self.ready_at = 0.0
        self.state = 0
        self.hung = False
        self.open = None
        self.data = 0
        self.errors = 0
        self.stop_unseen = False

    def configure(self, config):
        adc = config["adc"]
        dac_mode, dac_after = config["dac"]
        self.devices = {0x49: Device("dac", 0x49, dac_mode, dac_after, ramp_after=config["dac-ramp-after"])}
        if adc is not None:
            self.devices[adc] = Device("adc", adc, config["adc-mode"], config["adc-after"])
        self.devices[0x39] = Device("expander", 0x39, config["expander"], power_on={0: 0xFF})

    def busy(self):
        return self.open is not None or self.stop_unseen

    def pending(self):
        return not self.hung and self.board.now_ns >= self.ready_at

    def read(self, address):
        if address == 0x4008_9804:
            if self.pending() and self.state == 0:
                self.stop_unseen = False
            return (1 if self.pending() else 0) | (self.state << 1) | self.errors
        if address == 0x4008_9828:
            return self.data
        return self.board.stored(address)

    def write(self, address, value):
        board = self.board
        if address == 0x4008_9828:
            self.data = value & 0xFF
            return
        if address == 0x4008_9820:
            self.control(value)
            return
        if address == 0x4008_9804:
            self.errors &= ~value
            return
        if address == 0x4008_9800 and not value & 1:
            self.reset("master-off")
        board.store(address, value)
        board.emit(f"write {name_of(address)} {value:#x}")

    def reset(self, why):
        if self.open is not None:
            self.finish(f"abandoned-{why}")
        self.state = 0
        self.ready_at = self.board.now_ns
        self.stop_unseen = False

    def control(self, value):
        now = self.board.now_ns
        if value & 2:
            self.start(now)
        elif value & 1:
            self.carry_on(now)
        elif value & 4:
            if self.open is not None:
                self.finish(self.open["status"])
            self.state = 0
            self.ready_at = now + I2C_STOP_NS
            self.stop_unseen = True

    def start(self, now):
        address, reading = self.data >> 1, self.data & 1
        opening = self.open
        if opening is not None and not (reading and opening["address"] == address):
            self.finish(opening["status"])
            opening = None
        if opening is None:
            index = self.transaction_count
            self.transaction_count += 1
            hang_from = self.board.config["i2c-hang-from"]
            self.hung = hang_from is not None and index >= hang_from
            opening = {"address": address, "bytes": [], "status": "ack", "read": None}
            self.open = opening
        self.ready_at = now + I2C_BYTE_NS
        if not self.board.stored(FC3_CFG) & 1:
            opening["status"] = "nack-address"
            self.state = 3
            return
        device = self.devices.get(address)
        if device is not None and device.answers() and self.board.converter_powered(device):
            if reading:
                if opening["bytes"]:
                    device.pointer = opening["bytes"][0]
                if device.name == "expander" and device.pointer == 0:
                    self.data = self.board.expander_inputs()
                else:
                    self.data = device.registers.get(device.pointer, 0)
                opening["read"] = self.data
                self.state = 1
            else:
                self.state = 2
        else:
            opening["status"] = "nack-address"
            self.state = 3

    def carry_on(self, now):
        opening = self.open
        if opening is None or self.state != 2:
            return
        self.ready_at = now + I2C_BYTE_NS
        device = self.devices.get(opening["address"])
        opening["bytes"].append(self.data)
        if device is None or not device.answers() or not self.board.converter_powered(device):
            opening["status"] = f"nack-data@{len(opening['bytes']) - 1}"
            self.state = 4
            return
        if len(opening["bytes"]) == 1:
            device.pointer = self.data
        else:
            register = device.pointer
            device.registers[register] = self.data
            device.pointer = (device.pointer + 1) & 0xFF
            if device.name == "dac" and 16 <= register <= 21 and self.data != 0xFF:
                device.ramp_writes += 1
                if device.ramp_after is not None and device.ramp_writes >= device.ramp_after:
                    device.mode = "silent"
        self.state = 2

    def finish(self, status):
        opening, self.open = self.open, None
        if self.hung:
            status = "hung" if not status.startswith("abandoned") else status + "-hung"
        device = self.devices.get(opening["address"])
        if device is not None:
            device.transactions += 1
        address = opening["address"]
        if opening["read"] is not None:
            register = opening["bytes"][0] if opening["bytes"] else device.pointer
            self.board.emit(f"i2c read {address:#04x} reg {register:#04x} value {opening['read']:#04x} {status}")
        elif opening["bytes"]:
            listed = ",".join(f"{b:#04x}" for b in opening["bytes"])
            self.board.emit(f"i2c write {address:#04x} {listed} {status}")
        else:
            self.board.emit(f"i2c address {address:#04x} {status}")
        self.hung = False


def main():
    out = sys.stdout
    board = Board(out)
    uc = Uc(UC_ARCH_ARM, UC_MODE_THUMB | UC_MODE_MCLASS)
    uc.ctl_set_cpu_model(UC_CPU_ARM_CORTEX_M33)
    board.uc = uc
    uc.mem_map(RETURN_TRAP, 0x1000)
    uc.mem_map(RAM[0], RAM[1] - RAM[0])
    uc.mem_map(USB_RAM[0], USB_RAM[1] - USB_RAM[0])
    uc.mem_map(PPB[0], SCS - PPB[0])
    uc.mem_map(SCS + 0x1000, PPB[1] - SCS - 0x1000)

    def word_access(read_word, offset_base):
        def reader(u, offset, size, data):
            address = offset_base + offset
            word = read_word(address & ~3)
            return (word >> (8 * (address & 3))) & ((1 << (8 * size)) - 1)
        return reader

    def peripheral_read(address):
        if FLASH_CONTROLLER[0] <= address < FLASH_CONTROLLER[1]:
            board.emit(f"flash-controller read {address:#x}")
        if not board.guard(address, "read"):
            return 0
        return board.read_peripheral(address)

    def peripheral_write(u, offset, size, value, data):
        address = PERIPHERALS + offset
        if FLASH_CONTROLLER[0] <= address < FLASH_CONTROLLER[1]:
            board.emit(f"flash-controller write {address:#x} {value:#x}")
        if not board.guard(address, "write"):
            return
        if board.bus_fault_due(address):
            return
        if size < 4:
            shift = 8 * (address & 3)
            mask = ((1 << (8 * size)) - 1) << shift
            if 0x4008_C000 <= address < 0x4008_C040:
                board.write_peripheral(address, value)
                return
            value = (board.stored(address) & ~mask) | ((value << shift) & mask)
            address &= ~3
        board.write_peripheral(address, value)

    def secure_read(address):
        board.emit(f"secure-alias read {address:#x}")
        if 0x5003_4000 <= address < 0x5003_5000:
            board.emit(f"flash-controller read {address:#x}")
        board.guard(address, "read")
        return 0

    def secure_write(u, offset, size, value, data):
        address = SECURE_PERIPHERALS + offset
        board.emit(f"secure-alias write {address:#x} {value:#x}")
        if 0x5003_4000 <= address < 0x5003_5000:
            board.emit(f"flash-controller write {address:#x} {value:#x}")
        board.guard(address, "write")

    def flash_read(address):
        board.emit(f"flash-read {address:#x}")
        if board.guard(address, "read") and board.config["factory-read-faults"]:
            board.bus_fault_on_read(address)
        return board.config["flash-words"].get(address, 0)

    def flash_write(u, offset, size, value, data):
        board.emit(f"flash-write {offset:#x} {value:#x}")
        board.guard(offset, "write")

    def scs_write(u, offset, size, value, data):
        board.write_scs(SCS + offset, value)

    uc.mmio_map(PERIPHERALS, PERIPHERAL_SPAN, word_access(peripheral_read, PERIPHERALS), None,
                peripheral_write, None)
    uc.mmio_map(SECURE_PERIPHERALS, PERIPHERAL_SPAN, word_access(secure_read, SECURE_PERIPHERALS), None,
                secure_write, None)
    uc.mmio_map(0, FLASH_SPAN, word_access(flash_read, 0), None, flash_write, None)
    uc.mmio_map(SCS, 0x1000, word_access(board.read_scs, SCS), None, scs_write, None)

    def above_the_stack(u, access, address, size, value, data):
        board.guard(address, "write" if access == UC_HOOK_MEM_WRITE else "read")

    uc.hook_add(UC_HOOK_MEM_READ | UC_HOOK_MEM_WRITE, above_the_stack, begin=RAM_ABOVE_THE_STACK,
                end=RAM[1] - 1)

    entry = None
    for behavior, label, text in BEHAVIORS:
        out.write(f"0 behavior {behavior} {label} | {text}\n")
    for line in sys.stdin:
        words = line.split()
        if not words:
            continue
        verb, rest = words[0], words[1:]
        if verb == "load":
            load(uc, " ".join(rest))
        elif verb == "entry":
            entry = int(rest[0], 0)
        elif verb == "breadcrumb":
            address = int(rest[0], 0)

            def stored_magic(u, access, at, size, value, data):
                board.emit(f"breadcrumb-magic {value:#x}")

            def stored_record(u, access, at, size, value, data):
                board.emit(f"breadcrumb {value:#x}")
                for irq, count in sorted(board.delivered.items()):
                    board.emit(f"irq {irq} delivered {count} at-breadcrumb")

            uc.hook_add(UC_HOOK_MEM_WRITE, stored_magic, begin=address, end=address + 3)
            uc.hook_add(UC_HOOK_MEM_WRITE, stored_record, begin=address + 4, end=address + 7)
        elif verb == "host":
            board.config["host"] = rest[0]
        elif verb == "adc":
            board.config["adc"] = None if rest[0] == "none" else int(rest[0], 16)
            if rest[1:2] == ["silent-after"]:
                board.config["adc-mode"] = "silent-after"
                board.config["adc-after"] = int(rest[2], 0)
            else:
                board.config["adc-mode"] = "answers"
        elif verb == "dac":
            if rest[0] == "ramp-after":
                board.config["dac-ramp-after"] = int(rest[1], 0)
            else:
                board.config["dac"] = (rest[0], int(rest[1], 0) if len(rest) > 1 else 0)
        elif verb == "expander":
            board.config["expander"] = rest[0]
        elif verb == "buttons":
            board.config["buttons"].append((int(rest[1], 0) * 1_000_000, int(rest[2], 0) * 1_000_000, int(rest[0], 16)))
        elif verb == "converters-need-power":
            board.config["converters-need-power"] = True
        elif verb == "knobs":
            board.config["knobs"] = [int(word, 16) & 0xFFFF for word in rest[:5]]
        elif verb == "fc4-dma":
            board.fc4["mode"] = rest[0]
        elif verb == "analog-bus":
            board.config["analog-bus"] = rest[0]
        elif verb == "crystal":
            board.config["crystal"] = rest[0]
        elif verb == "pmc":
            for address, word in zip((0x4002_0010, 0x4002_0014, 0x4002_001C), rest[:3]):
                board.store(address, int(word, 0))
        elif verb == "pmc-ignores-writes":
            board.config["pmc-ignores-writes"] = True
        elif verb == "flash-word":
            board.config["flash-words"][int(rest[0], 0) & ~3] = int(rest[1], 0)
        elif verb == "pll-lock":
            board.config["pll-lock"] = rest[0]
            if rest[0] == "after-ms":
                board.config["pll-lock-ms"] = float(rest[1])
        elif verb == "phy-pll-lock":
            board.config["phy-pll-lock"] = rest[0]
        elif verb == "pll0-lock":
            if rest[0] == "never-after":
                board.config["pll0-lock"] = "never-after"
                board.config["pll0-lock-after"] = int(rest[1], 0)
            elif rest[0] == "after-ms":
                board.config["pll0-lock"] = "after-ms"
                board.config["pll0-lock-ms"] = float(rest[1])
            else:
                board.config["pll0-lock"] = rest[0]
        elif verb == "i2c-hang-from":
            board.config["i2c-hang-from"] = int(rest[0], 0)
        elif verb == "bus-fault-on-write":
            board.config["bus-fault"] = (int(rest[0], 0), int(rest[1], 0))
        elif verb == "plant-store":
            board.config["plant-store"] = (int(rest[0], 0), int(rest[1], 0) * 1_000_000)
        elif verb == "handover-write":
            board.config["handover-writes"].append((int(rest[0], 0), int(rest[1], 0), int(rest[2], 0) * 1_000_000))
        elif verb == "watchdog-inherited":
            protect = {"running": 0, "locked": 0x10}[rest[0]]
            board.inherit_watchdog(0x3 | protect, int(rest[1], 0), int(rest[2], 0))
        elif verb == "factory-read-faults":
            board.config["factory-read-faults"] = True
        elif verb == "mpu-regions":
            board.config["mpu-regions"], board.config["mpu-write-locked"] = int(rest[0], 0), rest[1:] == ["write-locked"]
        elif verb == "main-clock":
            board.store(0x4000_0280, int(rest[0], 0))
            board.store(0x4000_0284, int(rest[1], 0))
            if len(rest) > 2:
                board.store(0x4000_0380, int(rest[2], 0))
                board.store(0x4000_0400, int(rest[3], 0))
            board.select_main_clock(always_report=True)
        elif verb == "ahbclkctrl":
            board.store(0x4000_0200 + 4 * int(rest[0], 0), int(rest[1], 0))
        elif verb == "run":
            board.i2c.configure(board.config)
            run(board, entry, int(rest[0], 0) * 1_000_000)
        elif verb == "probe-fence":
            address, access = int(rest[0], 0), rest[1]
            verdict = board.mpu_decides(address, access, ignoring_priority=True)
            board.emit(f"mpu-probe {address:#x} {access} {verdict.split(' ')[0]}")
        else:
            sys.exit(f"plan line not understood: {line.strip()}")
    board.flush(float("inf"))


def run(board, entry, limit_ns):
    uc = board.uc
    uc.reg_write(UC_ARM_REG_SP, RAM_ABOVE_THE_STACK)
    pc = entry
    while board.ended is None:
        if board.now_ns >= limit_ns:
            board.end("time-limit")
            break
        try:
            uc.emu_start(pc | 1, RETURN_TRAP, count=CHUNK)
        except UcError as error:
            board.end(f"stopped {error} at {uc.reg_read(UC_ARM_REG_PC):#x}".replace(" ", "-"))
            break
        if board.ended is not None:
            break
        board.advance_cycles(CHUNK)
        if board.ended is not None:
            break
        pc = board.take_exception() if board.exception is not None else uc.reg_read(UC_ARM_REG_PC)
        if board.tick_pending():
            pc = call_interrupt(board, pc, TICK_IRQ)
            if board.ended is not None:
                break
        if board.usb_frame_pending():
            pc = call_interrupt(board, pc, USB_IRQ)
            if board.ended is not None:
                break
        writes = board.config["handover-writes"]
        while writes and board.now_ns >= writes[0][2]:
            address, value, _ = writes.pop(0)
            uc.mem_write(address, struct.pack("<I", value))
            board.emit(f"handover-write {address:#x} {value:#x}")
        plant = board.config["plant-store"]
        if plant is not None and board.now_ns >= plant[1]:
            board.config["plant-store"] = None
            uc.mem_write(PLANTED_STORE, PLANTED_STORE_CODE + struct.pack("<I", plant[0]))
            board.emit(f"planted-store {plant[0]:#x}")
            pc = PLANTED_STORE
    for irq, count in sorted(board.delivered.items()):
        board.emit(f"irq {irq} delivered {count}")
    board.emit(f"stopped-at {uc.reg_read(UC_ARM_REG_PC):#x}")


def call_interrupt(board, pc, irq):
    uc = board.uc
    saved = [uc.reg_read(register) for register in SAVED_REGISTERS]
    handler, = struct.unpack("<I", uc.mem_read(board.scb[0xE000_ED08] + 4 * (16 + irq), 4))
    uc.reg_write(UC_ARM_REG_SP, (saved[13] - 0x68) & ~7)
    uc.reg_write(UC_ARM_REG_LR, RETURN_TRAP | 1)
    board.delivered[irq] = board.delivered.get(irq, 0) + 1
    at = handler
    while board.ended is None:
        try:
            uc.emu_start(at | 1, RETURN_TRAP, count=INTERRUPT_CHUNK)
        except UcError as error:
            board.end(f"stopped {error} in irq {irq} at {uc.reg_read(UC_ARM_REG_PC):#x}".replace(" ", "-"))
            return pc
        board.advance_cycles(INTERRUPT_CHUNK)
        if board.exception is not None:
            return board.take_exception()
        at = uc.reg_read(UC_ARM_REG_PC)
        if at == RETURN_TRAP:
            break
    for register, value in zip(SAVED_REGISTERS, saved):
        uc.reg_write(register, value)
    return pc


if __name__ == "__main__":
    main()

