#![no_std]
#![no_main]

extern crate init;
use ::serial::prelude::*;
use system::System;

#[no_mangle]
fn bluemetal(system: &System) -> ! {
    println!("{:?}", system);
    println!("{:?}", system.memory());
    todo!();
}
