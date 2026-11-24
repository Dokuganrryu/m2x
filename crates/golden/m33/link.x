MEMORY
{
  FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 1M
  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 256K
}
SECTIONS
{
  .text : { *(.text .text.*) } > FLASH
  .rodata : { *(.rodata .rodata.*) } > FLASH
  .data : { *(.data .data.*) } > RAM AT > FLASH
  .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
  /DISCARD/ : { *(.ARM.exidx .ARM.exidx.* .ARM.extab.*) }
}
