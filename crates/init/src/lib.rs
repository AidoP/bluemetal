#![no_std]
//! The Bluemetal initialisation entry point.
//!
//! This crate handles the early-boot process before entering the kernel.

extern crate panic;

mod trap;

extern "Rust" {
    /// The main entry point to the kernel.
    fn bluemetal(hart_id: usize) -> !;
}

#[no_mangle]
extern "C" fn init(hart_id: usize) -> ! {
    ::serial::init();
    unsafe { bluemetal(hart_id) }
}
