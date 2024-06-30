#![no_std]
//! The Bluemetal initialisation entry point.
//!
//! This crate handles the early-boot process before entering the kernel.

extern crate panic;

extern "Rust" {
    /// The main entry point to the kernel.
    fn bluemetal(hart_id: usize) -> !;
}

#[no_mangle]
extern "C" fn init(hart_id: usize) -> ! {
    ::serial::init();
    unsafe { bluemetal(hart_id) }
}

#[no_mangle]
extern "C" fn trap(mepc: usize, mcause: usize) -> ! {
    panic!("hardware interrupt!\nmepc: 0x{mepc:016x}, mcause: 0x{mcause:016x}");
}
