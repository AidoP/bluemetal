.section .text, "ax", %progbits

.global _trap
_trap:
    call trap
