.section .entry, "ax", %progbits
# Machine-mode entry point for RISC-V

.global _init
_init:
    // save hart_id in a0 until `init()`
    csrr a0, mhartid
    // use only 1 hart
    bnez a0, _hang

    // `a1` **may** contain the address of a DTB
    // attempt to test it for the DTB header magic
    la t0, 1f
    csrw mtvec, t0
    lw t0, 0(a1)
    lw t1, dtb_magic
    beq t1, t0, 2f
    j 1f
dtb_magic:
    .byte 0xd0, 0x0d, 0xfe, 0xed
    .align 4
1:
    // `a1` is not a DTB so leave it as `None`
    mv a1, zero
2:

    csrw mscratch, a1

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
