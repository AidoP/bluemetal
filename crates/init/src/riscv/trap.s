.section .text, "ax", %progbits

// Initial trap vector for diagnosing early boot issues.
.align 4
.global _trap_early_panic
_trap_early_panic:
    // reset over the old initial stack as it is probably broken anyway

    // disable interrupts
    csrw mie, zero

    // zero out bss
    lla t0, _bss_start
    lla t1, _bss_end
    bgeu t0, t1, 2f
1:
    sd zero, 0(t0)
    addi t0, t0, 8
    bltu t0, t1, 1b
2:
    // reset stack
    lla sp, _stack_end

    // prevent an infinite kernel panic loop
    la a0, _hang
    csrw mtvec, a0

    csrr a0, mepc
    csrr a1, mcause
    j trap_early_panic

.align 4
.global _hang
_hang:
    wfi
    j _hang
