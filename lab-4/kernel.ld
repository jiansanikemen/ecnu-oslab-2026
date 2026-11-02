OUTPUT_ARCH(riscv)
ENTRY(_entry)

SECTIONS
{
    . = 0x80000000;

    stext = .;
    .text : {
        *(.text.entry)

        . = ALIGN(4096);
        strampoline = .;

        KEEP(*(.text.trampoline))

        ASSERT(
            . - strampoline <= 4096,
            "user trampoline exceeds one page"
        );

        . = ALIGN(4096);
        etrampoline = .;

        *(.text .text.*)
    }
    . = ALIGN(4096);
    etext = .;

    srodata = .;
    .rodata : { *(.rodata .rodata.*) }
    . = ALIGN(4096);
    erodata = .;

    sdata = .;
    .data : { *(.data .data.*) }
    . = ALIGN(4096);
    edata = .;

    sbss = .;
    .bss : {
        *(.bss.stack)
        *(.bss .bss.* COMMON)
    }
    . = ALIGN(4096);
    ebss = .;
    ekernel = .;

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
    }
}
