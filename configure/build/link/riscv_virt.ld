OUTPUT_ARCH(riscv)
ENTRY(_rt0)

MEMORY {
    ram (rwx): ORIGIN = 0x80000000, LENGTH = 0x80000
}

SECTIONS {
    . = ALIGN(0x1000);
    .text : {
        *(.entry)
        *(.text)
        *(.text.*)
    } > ram

    . = ALIGN(0x1000);
    .rodata : {
        *(.rodata)
        *(.rodata.*)
    } > ram

    . = ALIGN(0x10);
    .eh_frame : {
        PROVIDE(_eh_frame = .);
        *(.eh_frame)
        PROVIDE(_eh_frame_len = SIZEOF(.eh_frame));
    } > ram

    . = ALIGN(0x10);
    .eh_frame_hdr : {
        PROVIDE(_eh_frame_hdr = .);
        *(.eh_frame_hdr)
    } > ram

    .data : ALIGN(0x1000) {
        *(.data .data.*)
        . = ALIGN(16);
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.sdata .sdata.*)
    } > ram

    . = ALIGN(0x1000);
    .bss : {
        PROVIDE(_bss_start = .);
        *(.bss)
        *(.bss.*)
        . = ALIGN(8);
        PROVIDE(_bss_end = .);
    } > ram

    . = ALIGN(0x1000);
    .stack (NOLOAD) : {
        . = . + 0x10000;
        PROVIDE(_stack_end = .);
    } > ram
}
