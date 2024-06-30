.section .text, "ax", %progbits

// an mtvec target must be 4-byte aligned
.align 4
.global _trap
_trap:
    // todo: save registers and fix / allocate new stack
    // for now the _trap will always be entered while the stack is usable, but
    // we will corrupt it so can never return - kernel panic instead.
    csrr a0, mepc
    csrr a1, mcause
    j trap
