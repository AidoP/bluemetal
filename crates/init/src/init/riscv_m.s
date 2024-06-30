.section .entry, "ax", %progbits
# Machine-mode entry point for RISC-V

.global _rt0
_rt0:
    // use only 1 hart
    csrr a0, mhartid
    bnez a0, _hang

    // set machine-mode memory access and machine mode interrupt enable
    li t0, (0b11 << 11) | (0b1 < 7) | (0b1 << 3)
    csrw mstatus, t0

    // disable interrupts
    csrw mie, zero

    // clear interrupts
    csrw mip, zero

    // zero out bss
    lla a6, _bss_start
    lla a7, _bss_end
    bgeu a6, a7, 2f
1:
    sd zero, 0(a6)
    addi a6, a6, 8
    bltu a6, a7, 1b
2:

    // set trap vector
    la a6, _trap
    csrw mtvec, a6

    // set stack pointer
    lla sp, _stack_end

    // set global pointer
.option push
.option norelax
    la  gp, __global_pointer$
.option pop

    // get hart ID
    csrr a0, mhartid

    // jump to init()
    j init

.global _hang
_hang:
    wfi
    j _hang
