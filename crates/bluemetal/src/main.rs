#![no_std]
#![no_main]

extern crate panic;

pub mod trap;

#[no_mangle]
extern "C" fn entry(hart_id: usize) -> ! {
    let serial = serial::sifive_uart(0).unwrap();
    let _ = writeln!(serial, "Hello, Hart {hart_id}!");
    todo!();
}
