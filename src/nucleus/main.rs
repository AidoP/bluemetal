#![no_std]
#![no_main]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

core::arch::global_asm!(
    ".global _start",
    "_start:",
    "   j _start",
);
