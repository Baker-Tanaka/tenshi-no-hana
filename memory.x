MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x100

    /* RP2040 SRAM total: 256K (SRAM0-3) + 4K SCRATCH_X + 4K SCRATCH_Y = 264K
     * All banks are contiguous (0x20000000 - 0x20041FFF) and safe to treat as
     * one block unless striped DMA access is needed. Using 264K gives an extra
     * 8K of headroom for the stack (flip-link places .bss/.data at the top). */
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}

EXTERN(BOOT2_FIRMWARE)

SECTIONS {
    /* ### Boot loader */
    .boot2 ORIGIN(BOOT2) :
    {
        KEEP(*(.boot2));
    } > BOOT2
} INSERT BEFORE .text;