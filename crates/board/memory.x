/* The loader copies the image to 0x20000000 and takes word 0 as SP 0x20030000; the image region ends at 0x20029400, where RAM starts. */
MEMORY
{
  FLASH : ORIGIN = 0x20000000, LENGTH = 0x29400
  RAM   : ORIGIN = 0x20029400, LENGTH = 0x6C00
}

_stack_start = 0x20030000;

/* At least 8 KB of stack, and an image no larger than the 168,532-byte region the loader copies. */
ASSERT(_stack_end + 8192 <= _stack_start, "board: less than 8 KB of stack remains below 0x20030000");
ASSERT(__sidata + (__edata - __sdata) - ORIGIN(FLASH) <= 168532, "board: the image is larger than the 168532-byte image region");
ASSERT(SIZEOF(.gnu.sgstubs) == 0, "board: the image exports no secure gateway veneers, so the image ends with the .data load copy");
/* The breadcrumb is the whole no-init section, above the zeroed .bss and right under the stack, so a warm reset keeps it and the stack size does not move. */
ASSERT(BREADCRUMB == __suninit && __euninit - __suninit == 8 && BREADCRUMB >= __ebss && __euninit == _stack_end, "board: the breadcrumb is not the eight no-init bytes at the stack floor");

/* The DMA channel table and the LCD word buffers are uninitialised, so they take no image bytes.
   They sit in the image region right after the loaded image, so the FLASH region bounds them. */
SECTIONS
{
  .panel_buffers (NOLOAD) : ALIGN(512)
  {
    *(.panel_buffers .panel_buffers.*);
  } > FLASH
}
INSERT AFTER .gnu.sgstubs;

/* The vector table the image runs on once its contexts are installed. Armv8-M VTOR needs it 512-byte aligned,
   so it opens RAM, where the alignment costs no padding that a planted build could absorb. It is filled after the hand-over
   quiesce and holds no image bytes. */
SECTIONS
{
  .running_vectors (NOLOAD) : ALIGN(512)
  {
    *(.running_vectors .running_vectors.*);
  } > RAM
}
INSERT BEFORE .data;
