.section .entry, "ax", %progbits
# Machine-mode entry point for RISC-V

.global _rt0
_rt0:
    // disable interrupts
    csrw mie, zero

    // clear interrupts
    csrw mip, zero

    // use only 1 hart
    csrr a0, mhartid
    bnez a0, _hang

    // zero out bss
    lla a6, _bss_start
    lla a7, _bss_end
1:
    sd zero, 0(a6)
    addi a6, a6, 8
    bltu a6, a7, 1b

    // set trap vector
    la a6, _trap
    csrw mtvec, a6

    // set stack pointer
    lla sp, _stack_end

    // call Rust init
    j init

.global _hang
_hang:
    wfi
    j _hang
