"""Run a plan of calls on an emulated Cortex-M33 and print the executed addresses of the traced ones.

Usage: python -I emulate.py < plan

`cargo xtask budget` writes the plan and reads the output. This script prices nothing and knows no
symbol. Plan lines:
  map <base> <size>            map zeroed memory
  rom <base> <size>            map zeroed memory that refuses every write
  load <elf>                   copy every PT_LOAD segment to its address
  stack <top>                  stack pointer for each call
  boot <entry> <instructions> [<loop start> <loop end>]
                               run from entry for that many instructions, then one instruction at a
                               time, at most as many again, until the pc is inside the loop range
                               when one is given, print `booted <pc>`, then call below its stack. A
                               boot that stops early on an instruction it cannot execute prints
                               where and ends the plan
  poke8 <address> <byte>       write one byte
  call <entry> [arg...]        run a function to its return, untraced
  trace <label> <entry> [arg...]  the same, printing `trace <label>`, one hex address per executed
                               instruction with its byte size, then `end`
An access outside the mapped memory stops the run and names the address and the instruction.
Needs unicorn from PyPI. ELF bytes are data.
"""
import struct
import sys

from unicorn import UC_ARCH_ARM, UC_HOOK_CODE, UC_HOOK_MEM_INVALID, UC_MEM_FETCH_PROT
from unicorn import UC_MEM_FETCH_UNMAPPED, UC_MEM_READ_PROT, UC_MEM_READ_UNMAPPED, UC_MEM_WRITE_PROT
from unicorn import UC_MEM_WRITE_UNMAPPED, UC_MODE_MCLASS, UC_MODE_THUMB, UC_PROT_EXEC, UC_PROT_READ
from unicorn import Uc, UcError
from unicorn.arm_const import UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2
from unicorn.arm_const import UC_ARM_REG_R3, UC_ARM_REG_SP, UC_CPU_ARM_CORTEX_M33

RETURN_TRAP = 0x10000000
ARGUMENT_REGISTERS = (UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3)
EXCEPTION_FRAME_ROOM = 0x100
ACCESS_NAMES = {
    UC_MEM_READ_UNMAPPED: "read of unmapped",
    UC_MEM_WRITE_UNMAPPED: "write to unmapped",
    UC_MEM_FETCH_UNMAPPED: "fetch from unmapped",
    UC_MEM_READ_PROT: "read of protected",
    UC_MEM_WRITE_PROT: "write to read-only",
    UC_MEM_FETCH_PROT: "fetch from non-executable",
}


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
    stack = [0]
    out = sys.stdout
    refused = []

    def refuse(u, access, address, size, value, data):
        refused.append(f"{ACCESS_NAMES.get(access, access)} memory at {address:#x}, {size} bytes, "
                       f"by the instruction at {u.reg_read(UC_ARM_REG_PC):#x}")
        return False

    uc.hook_add(UC_HOOK_MEM_INVALID, refuse)

    def run(begin, until, count=0):
        try:
            uc.emu_start(begin, until, count=count)
        except UcError as error:
            if refused:
                sys.exit(f"{error}: {refused[-1]}")
            return error
        return None

    def stop(error):
        sys.exit(f"{error} at {uc.reg_read(UC_ARM_REG_PC):#x}")

    def call(entry, arguments, hook=None):
        for register, value in zip(ARGUMENT_REGISTERS, arguments):
            uc.reg_write(register, value)
        uc.reg_write(UC_ARM_REG_SP, stack[0])
        uc.reg_write(UC_ARM_REG_LR, RETURN_TRAP | 1)
        handle = uc.hook_add(UC_HOOK_CODE, hook) if hook else None
        uc.ctl_flush_tb()
        try:
            error = run(entry | 1, RETURN_TRAP)
            if error is not None:
                stop(error)
        finally:
            if handle is not None:
                uc.hook_del(handle)
                uc.ctl_flush_tb()

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
        elif verb == "stack":
            stack[0] = int(rest[0], 0)
        elif verb == "boot":
            uc.reg_write(UC_ARM_REG_SP, stack[0])
            budget = int(rest[1], 0)
            stopped = run(int(rest[0], 0) | 1, RETURN_TRAP, budget)
            if stopped is None and len(rest) == 4:
                start, end = int(rest[2], 0), int(rest[3], 0)
                for _ in range(budget):
                    pc = uc.reg_read(UC_ARM_REG_PC)
                    if start <= pc < end:
                        break
                    stopped = run(pc | 1, RETURN_TRAP, 1)
                    if stopped is not None:
                        break
            out.write(f"booted {uc.reg_read(UC_ARM_REG_PC):x}\n")
            if stopped is not None:
                return
            stack[0] = (uc.reg_read(UC_ARM_REG_SP) - EXCEPTION_FRAME_ROOM) & ~7
        elif verb == "poke8":
            uc.mem_write(int(rest[0], 0), bytes([int(rest[1], 0)]))
        elif verb == "call":
            call(int(rest[0], 0), [int(word, 0) for word in rest[1:]])
        elif verb == "trace":
            out.write(f"trace {rest[0]}\n")
            call(int(rest[1], 0), [int(word, 0) for word in rest[2:]],
                 lambda u, address, size, d: out.write(f"{address:x} {size}\n"))
            out.write("end\n")
        else:
            sys.exit(f"plan line not understood: {line.strip()}")


if __name__ == "__main__":
    main()
