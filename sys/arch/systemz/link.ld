OUTPUT_ARCH(s390x)
ENTRY(start)

PHDRS {
    text PT_LOAD AT(0);
}

SECTIONS {

    .text 0 : {
        *(.start)
        *(.text .text.*)
    } :text

    .rodata : {
        *(.rodata)
        *(.rodata.*)
    } :text

    .data : {
        *(.data .data.*)
        . = ALIGN(16);
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.sdata .sdata.*)
    } :text

    .bss : ALIGN(16) {
        PROVIDE(__bss_start = .);
        *(.bss)
        *(.bss.*)
        . = ALIGN(8);
        PROVIDE(__bss_end = .);
    } :text

    . = ALIGN(4096);
    __memory_start = .;
}
