.section .text, "ax", %progbits

.align 4
.global _trap_early_panic
_trap_early_panic:
    wfi
    j _trap_early_panic
