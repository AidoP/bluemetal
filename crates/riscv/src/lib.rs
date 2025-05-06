#![no_std]
#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]

use core::arch::asm;

/// Returns the contents of the `mhartid` CSR.
///
/// # Safety
/// It is undefined behaviour to call this outside of machine mode, if the CSR
/// registers are unavailable, or if the CSR ISA extension is not enabled.
#[inline]
pub unsafe fn mhartid() -> usize {
    let hart_id;
    asm!(
        "csrr {hart_id}, mhartid",
        hart_id = out(reg) hart_id,
    );
    hart_id
}

/// Returns the contents of the `misa` CSR.
///
/// # Safety
/// It is undefined behaviour to call this outside of machine mode, if the CSR
/// registers are unavailable, or if the CSR ISA extension is not enabled.
#[inline]
pub unsafe fn misa() -> usize {
    let isa;
    asm!(
        "csrr {isa}, misa",
        isa = out(reg) isa,
    );
    isa
}

/// Returns the contents of the `mscratch` CSR.
///
/// # Safety
/// It is undefined behaviour to call this outside of machine mode, if the CSR
/// registers are unavailable, or if the CSR ISA extension is not enabled.
#[inline]
pub unsafe fn mscratch() -> usize {
    let scratch;
    asm!(
        "csrr {scratch}, mscratch",
        scratch = out(reg) scratch,
    );
    scratch
}
