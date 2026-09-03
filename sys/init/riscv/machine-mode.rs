#![no_std]

core::arch::global_asm!(
    include_str!("machine-mode.s"),
    sym init,
);

extern "C" fn init() {

}
