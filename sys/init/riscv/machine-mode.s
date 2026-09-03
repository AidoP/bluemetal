.global _init
_init:
    csrr a0, mhartid
    bnez a0, _init_wait
    j {}

_init_wait:
    wfi
    j _init_wait
