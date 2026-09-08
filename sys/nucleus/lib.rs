#![no_std]

//pub mod console;
//pub mod spinlock;

pub fn main() -> ! {
    unreachable!();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    unsafe {
        // give control to the debugger or interrupt handler
        // core::arch::asm!("ebreak", options(noreturn));
    }
    loop {}
}
