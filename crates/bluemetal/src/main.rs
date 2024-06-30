#![no_std]
#![no_main]

extern crate init;
use ::serial::prelude::*;

#[no_mangle]
fn bluemetal(hart_id: usize) -> ! {
    println!("Hello, Hart {hart_id}!");

    todo!();
}
