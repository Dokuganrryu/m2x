"""Start the linked image on an emulated Cortex-M33 from an inherited state and print every write.

Usage: python -I emulate.py < plan

`cargo xtask hand-over` writes the plan and judges the output. This script judges nothing and knows
no symbol. Peripherals are plain memory, with these exceptions, each applied after the instruction
that wrote:
  NVIC ICER and ICPR words clear the matching ISER and ISPR bits.
  ICSR PENDSTCLR and PENDSVCLR clear PENDSTSET and PENDSVSET.
  DMA0 ENABLECLR0 clears the matching ENABLESET0 bits, so ENABLESET0 reads as the enabled channels.
  With `countflag`, every read of SYST_CSR sees COUNTFLAG set, one elapsed reload per read.
Exceptions are modelled. Before each instruction until `until`, while PRIMASK is clear, a pending
SysTick (ICSR PENDSTSET), a pending PendSV, or an IRQ whose ISER and ISPR bits are both set is taken:
the pending bit clears, an eight-word frame is pushed below SP, the handler named by the vector at
VTOR runs until it returns, and the registers are restored. SysTick goes first, then PendSV, then
IRQs by priority byte and number.

Plan lines:
  map <base> <size>            map zeroed memory
  rom <base> <size>            map zeroed memory that refuses every write
  load <elf>                   copy every PT_LOAD segment to its address
  poke32 <address> <word>      write one word of inherited state
  poke8 <address> <byte>       write one byte of inherited state
  sp <value>                   the inherited stack pointer
  primask <0|1>                the inherited PRIMASK
  countflag                    model SysTick COUNTFLAG as described above
  run <entry> <until> <most instructions>
  continue <instructions>      after `until`, run on with no exception model
Output lines:
  write <index> <pc> <address> <size> <value> <context>   context is startup, frame<n>,
                               exception<n> or after; n is the exception number
  exception <n> <handler> <index>
  return <n> <index>           followed by a state line
  noreturn <n> <pc>
  reached <pc> <index>
  stopped <pc> <index> <reason>
  state <name>=<hex> ...       PRIMASK and the modelled registers when `until` is reached
Needs unicorn from PyPI. ELF bytes are data.
"""
import struct
import sys

from unicorn import UC_ARCH_ARM, UC_HOOK_MEM_INVALID, UC_HOOK_MEM_READ, UC_HOOK_MEM_WRITE
from unicorn import UC_MODE_MCLASS, UC_MODE_THUMB, UC_PROT_EXEC, UC_PROT_READ, Uc, UcError
from unicorn.arm_const import UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_PRIMASK, UC_ARM_REG_R0
from unicorn.arm_const import UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3, UC_ARM_REG_R4
from unicorn.arm_const import UC_ARM_REG_R5, UC_ARM_REG_R6, UC_ARM_REG_R7, UC_ARM_REG_R8
from unicorn.arm_const import UC_ARM_REG_R9, UC_ARM_REG_R10, UC_ARM_REG_R11, UC_ARM_REG_R12
from unicorn.arm_const import UC_ARM_REG_SP, UC_ARM_REG_XPSR, UC_CPU_ARM_CORTEX_M33

RETURN_TRAP = 0x10000000
PERIPHERALS = 0x40000000
HANDLER_BUDGET = 20000
SAVED = (UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3, UC_ARM_REG_R4, UC_ARM_REG_R5,
         UC_ARM_REG_R6, UC_ARM_REG_R7, UC_ARM_REG_R8, UC_ARM_REG_R9, UC_ARM_REG_R10, UC_ARM_REG_R11,
         UC_ARM_REG_R12, UC_ARM_REG_SP, UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_XPSR)
SYST_CSR = 0xE000E010
SYST_CSR_COUNTFLAG = 1 << 16
NVIC_ISER = 0xE000E100
NVIC_ICER = 0xE000E180
NVIC_ISPR = 0xE000E200
NVIC_ICPR = 0xE000E280
NVIC_IPR = 0xE000E400
NVIC_WORDS = 2
ICSR = 0xE000ED04
ICSR_PENDSTCLR, ICSR_PENDSTSET = 1 << 25, 1 << 26
ICSR_PENDSVCLR, ICSR_PENDSVSET = 1 << 27, 1 << 28
VTOR = 0xE000ED08
DMA0_ENABLESET0 = 0x40082020
DMA0_ENABLECLR0 = 0x40082028
SYSTICK_EXCEPTION = 15
PENDSV_EXCEPTION = 14
FIRST_IRQ_EXCEPTION = 16
STATE_WORDS = (
    ("iser0", NVIC_ISER), ("iser1", NVIC_ISER + 4), ("ispr0", NVIC_ISPR), ("ispr1", NVIC_ISPR + 4),
    ("icsr", ICSR), ("vtor", VTOR), ("syst_csr", SYST_CSR), ("dma0_ctrl", 0x40082000),
    ("dma0_enabled", DMA0_ENABLESET0), ("ctimer0_tcr", 0x40008004), ("usb_devcmdstat", 0x40094000),
    ("usb1_dev_clock", 0x40000208),
)


def load(uc, path):
    data = open(path, "rb").read()
    phoff, = struct.unpack_from("<I", data, 0x1C)
    phentsize, phnum = struct.unpack_from("<HH", data, 0x2A)
    for i in range(phnum):
        kind, offset, vaddr, _, filesz, _, _, _ = struct.unpack_from("<8I", data, phoff + i * phentsize)
        if kind == 1 and filesz:
            uc.mem_write(vaddr, data[offset:offset + filesz])


def main():
    uc = Uc(UC_ARCH_ARM, UC_MODE_THUMB | UC_MODE_MCLASS)
    uc.ctl_set_cpu_model(UC_CPU_ARM_CORTEX_M33)
    uc.mem_map(RETURN_TRAP, 0x1000)
    out = sys.stdout
    clock = {"index": 0, "context": "startup"}
    effects = []
    refused = []

    def word(address):
        return struct.unpack("<I", uc.mem_read(address, 4))[0]

    def set_word(address, value):
        uc.mem_write(address, struct.pack("<I", value & 0xFFFFFFFF))

    def on_write(u, access, address, size, value, data):
        if clock["context"] == "after" and address < PERIPHERALS:
            return
        out.write(f"write {clock['index']} {u.reg_read(UC_ARM_REG_PC):x} {address:x} {size} "
                  f"{value & 0xFFFFFFFF:x} {clock['context']}\n")
        effects.append((address, size, value))

    def on_refused(u, access, address, size, value, data):
        refused.append(f"access {access} to unmapped or read-only memory at {address:#x}")
        return False

    def apply_effects():
        for address, size, value in effects:
            if size != 4:
                continue
            for index in range(NVIC_WORDS):
                if address == NVIC_ICER + 4 * index:
                    set_word(NVIC_ISER + 4 * index, word(NVIC_ISER + 4 * index) & ~value)
                if address == NVIC_ICPR + 4 * index:
                    set_word(NVIC_ISPR + 4 * index, word(NVIC_ISPR + 4 * index) & ~value)
            if address == ICSR:
                cleared = (ICSR_PENDSTSET if value & ICSR_PENDSTCLR else 0) | \
                          (ICSR_PENDSVSET if value & ICSR_PENDSVCLR else 0)
                set_word(ICSR, value & ~(ICSR_PENDSTCLR | ICSR_PENDSVCLR | cleared))
            if address == DMA0_ENABLECLR0:
                set_word(DMA0_ENABLESET0, word(DMA0_ENABLESET0) & ~value)
        effects.clear()

    def countflag(u, access, address, size, value, data):
        set_word(SYST_CSR, word(SYST_CSR) | SYST_CSR_COUNTFLAG)

    def pending_exception():
        icsr = word(ICSR)
        if icsr & ICSR_PENDSTSET:
            return SYSTICK_EXCEPTION, lambda: set_word(ICSR, word(ICSR) & ~ICSR_PENDSTSET)
        if icsr & ICSR_PENDSVSET:
            return PENDSV_EXCEPTION, lambda: set_word(ICSR, word(ICSR) & ~ICSR_PENDSVSET)
        live = []
        for index in range(NVIC_WORDS):
            bits = word(NVIC_ISER + 4 * index) & word(NVIC_ISPR + 4 * index)
            for bit in range(32):
                if bits & (1 << bit):
                    irq = 32 * index + bit
                    live.append((uc.mem_read(NVIC_IPR + irq, 1)[0], irq))
        if not live:
            return None
        _, irq = min(live)
        pending = NVIC_ISPR + 4 * (irq // 32)
        return FIRST_IRQ_EXCEPTION + irq, lambda: set_word(pending, word(pending) & ~(1 << (irq % 32)))

    def step(pc):
        try:
            uc.emu_start(pc | 1, RETURN_TRAP, count=1)
        except UcError as error:
            return refused[-1] if refused else str(error)
        finally:
            apply_effects()
        return None

    def take(number, clear_pending):
        clear_pending()
        saved = [uc.reg_read(register) for register in SAVED]
        sp = (saved[13] - 32) & ~7
        clock["context"] = f"frame{number}"
        frame = saved[0:4] + [saved[12], saved[14], saved[15], saved[16]]
        for offset, value in enumerate(frame):
            on_write(uc, None, sp + 4 * offset, 4, value, None)
            set_word(sp + 4 * offset, value)
        effects.clear()
        handler = word(word(VTOR) + 4 * number)
        out.write(f"exception {number} {handler & ~1:x} {clock['index']}\n")
        clock["context"] = f"exception{number}"
        uc.reg_write(UC_ARM_REG_SP, sp)
        uc.reg_write(UC_ARM_REG_LR, RETURN_TRAP | 1)
        pc = handler & ~1
        for _ in range(HANDLER_BUDGET):
            if pc == RETURN_TRAP:
                break
            failure = step(pc)
            clock["index"] += 1
            if failure is not None:
                out.write(f"noreturn {number} {pc:x}\n")
                break
            pc = uc.reg_read(UC_ARM_REG_PC) & ~1
        else:
            out.write(f"noreturn {number} {pc:x}\n")
        if pc == RETURN_TRAP:
            out.write(f"return {number} {clock['index']}\n")
            state_line()
        for register, value in zip(SAVED, saved):
            uc.reg_write(register, value)
        clock["context"] = "startup"

    def state_line():
        fields = [f"primask={uc.reg_read(UC_ARM_REG_PRIMASK) & 1:x}"]
        fields += [f"{name}={word(address):x}" for name, address in STATE_WORDS]
        out.write("state " + " ".join(fields) + "\n")

    uc.hook_add(UC_HOOK_MEM_INVALID, on_refused)
    for line in sys.stdin:
        words = line.split()
        if not words:
            continue
        verb, rest = words[0], words[1:]
        if verb == "map":
            uc.mem_map(int(rest[0], 0), int(rest[1], 0))
        elif verb == "rom":
            uc.mem_map(int(rest[0], 0), int(rest[1], 0), UC_PROT_READ | UC_PROT_EXEC)
        elif verb == "load":
            load(uc, " ".join(rest))
        elif verb == "poke32":
            set_word(int(rest[0], 0), int(rest[1], 0))
        elif verb == "poke8":
            uc.mem_write(int(rest[0], 0), bytes([int(rest[1], 0)]))
        elif verb == "sp":
            uc.reg_write(UC_ARM_REG_SP, int(rest[0], 0))
        elif verb == "primask":
            uc.reg_write(UC_ARM_REG_PRIMASK, int(rest[0], 0))
        elif verb == "countflag":
            uc.hook_add(UC_HOOK_MEM_READ, countflag, begin=SYST_CSR, end=SYST_CSR + 3)
        elif verb == "run":
            uc.hook_add(UC_HOOK_MEM_WRITE, on_write)
            pc, until, budget = int(rest[0], 0) & ~1, int(rest[1], 0) & ~1, int(rest[2], 0)
            reached = False
            while clock["index"] < budget:
                if pc == until:
                    reached = True
                    break
                if not uc.reg_read(UC_ARM_REG_PRIMASK) & 1:
                    pending = pending_exception()
                    if pending is not None:
                        uc.reg_write(UC_ARM_REG_PC, pc | 1)
                        take(*pending)
                        continue
                failure = step(pc)
                clock["index"] += 1
                if failure is not None:
                    out.write(f"stopped {pc:x} {clock['index']} {failure}\n")
                    state_line()
                    return
                pc = uc.reg_read(UC_ARM_REG_PC) & ~1
            if not reached:
                out.write(f"stopped {pc:x} {clock['index']} the instruction budget ran out\n")
                state_line()
                return
            out.write(f"reached {pc:x} {clock['index']}\n")
            state_line()
        elif verb == "continue":
            clock["context"] = "after"
            pc = uc.reg_read(UC_ARM_REG_PC) & ~1
            try:
                uc.emu_start(pc | 1, RETURN_TRAP, count=int(rest[0], 0))
            except UcError as error:
                out.write(f"stopped {uc.reg_read(UC_ARM_REG_PC):x} {clock['index']} "
                          f"{refused[-1] if refused else error}\n")
            apply_effects()
        else:
            sys.exit(f"plan line not understood: {line.strip()}")


if __name__ == "__main__":
    main()
