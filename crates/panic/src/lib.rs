#![no_std]
#![allow(internal_features)]
#![feature(lang_items)]

#[lang = "eh_personality"]
#[no_mangle]
unsafe extern "C" fn rust_eh_personality() {}

#[no_mangle]
unsafe extern "C" fn _Unwind_Resume() {}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(serial) = serial::sifive_uart(0) {
        let _ = write!(
            serial,
"
╔═══════════════════╗
║ ⚠ Kernel Panic 🮲🮳 ║
╚═══════════════════╝
{info}
"
        );
    }
    loop {}
}
