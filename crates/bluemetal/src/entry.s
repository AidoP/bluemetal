.section .entry, "ax", %progbits

.global _rt0
_rt0:
    // Use only 1 hart
    // bnez a0, _hang

    // zero out bss
    lla a6, _bss_start
    lla a7, _bss_end
1:
    sd zero, 0(a6)
    addi a6, a6, 8
    bltu a6, a7, 1b

    // clear interrupts
    csrw sip, zero
    csrw sie, zero

    // set trap vector
    la a6, _trap
    csrw stvec, a6

    // set stack pointer
    lla sp, _stack_end
    // clear frame pointer
    li s0, 0

    // call entry point
    j entry

.global _hang
_hang:
    wfi
    j _hang
