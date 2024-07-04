.section .entry, "ax", %progbits
# Machine-mode entry point for RISC-V

.global _rt0
_rt0:
    // save hart_id in a0 until `init()`
    csrr a0, mhartid
    // use only 1 hart
    bnez a0, _hang

    // set early trap vector
    la t0, _trap_early_panic
    csrw mtvec, t0

    // set machine-mode memory access and machine mode interrupt enable
    li t0, (0b11 << 11) | (0b1 < 7) | (0b1 << 3)
    csrw mstatus, t0

    // disable interrupts
    csrw mie, zero

    // clear interrupts
    csrw mip, zero

    // zero out bss
    lla t0, _bss_start
    lla t1, _bss_end
    bgeu t0, t1, 2f
1:
    sd zero, 0(t0)
    addi t0, t0, 8
    bltu t0, t1, 1b
2:

    // set stack pointer
    lla sp, _stack_end

    // set global pointer
.option push
.option norelax
    la  gp, __global_pointer$
.option pop

    // init(hart_id: a0) -> !
    j init

.global _hang
_hang:
    wfi
    j _hang

// Initial trap vector for diagnosing early boot issues.
.align 4
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
    la a0, 1f
    csrw mtvec, a0

    csrr a0, mepc
    csrr a1, mcause
    j trap_early_panic

    // a second trap will just park the hart here as things are probably fucked
.align 4
1:
    wfi
    j 1b
