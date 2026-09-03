#![no_std]
#![no_main]

use core::{
    arch::{
        asm,
        global_asm,
    },
};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    halt(0xFE42);
}

global_asm!(
    include_str!("ipl.s"),
    main = sym main,
);

/// Enter disabled wait state with the PSW set to a code.
/// Halts execution.
pub fn halt(code: u16) -> ! {
    let psw = 0x000A0000_00000000u64 | u64::from(code);
    unsafe {
        asm!(
            "lpsw 0({})",
            in(reg_addr) &psw,
            options(noreturn),
        );
    }
}

unsafe extern "C" fn main(_memory_size: usize) -> ! {



    // PSW will be set to `42` in disabled wait state.
    // Try adding I/O next...
    halt(42)
}

#[repr(C)]
struct Ccw1 {
    code: u8,
    flags: u8,
    count: u16,
    data: u32,
}

#[repr(C)]
struct Orb {
    interrupt: u32,
    flags: u32,
    program: u32,
    css_priority: u8,
    _reserved1: u8,
    cu_priority: u8,
    _reserved2: u8,
    _reserved3: [u32; 4],
}
