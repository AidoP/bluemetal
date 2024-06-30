#![no_std]
#![allow(internal_features)]
#![feature(lang_items)]

#[lang = "eh_personality"]
#[no_mangle]
unsafe extern "C" fn rust_eh_personality() {}

#[no_mangle]
unsafe extern "C" fn _Unwind_Resume() {}

extern "C" {
    fn _hang() -> !;
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut out = ::serial::global().lock();
    let _ = writeln!(
        out,
"
╔═══════════════════╗
║ ⚠ Kernel Panic 🮲🮳 ║
╚═══════════════════╝
{info}
"
    );
    unsafe { _hang() }
}
