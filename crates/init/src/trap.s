.section .text, "ax", %progbits

.global _trap
_trap:
    wfi
    j _trap
