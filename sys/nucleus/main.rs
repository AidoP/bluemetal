#![no_std]
#![no_main]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

core::arch::global_asm!(
    ".global _start",
    "_start:",
    "   li a7,64",
    "   li a0,1",
    "   la a1,msg",
    "   li a2,12",
    "   ecall",
    "   li a7,93",
    "   li a0,0",
    "   ecall",
    "msg: .ascii \"hello world\\n\"",
    ".set len, 6",
);
