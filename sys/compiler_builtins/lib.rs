#![allow(internal_features)]
#![compiler_builtins]
#![feature(compiler_builtins)]
#![no_std]
#![no_builtins]

#[allow(suspicious_runtime_symbol_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    while len > 0 {
        unsafe { dst.write(src.read()) };
    }
    return dst;
}
