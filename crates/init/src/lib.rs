#![no_std]
//! The Bluemetal initialisation entry point.
//!
//! This crate handles the early-boot process before entering the kernel.

#[cfg(feature = "dtb")]
use device_tree::Dtb;

extern crate panic;

mod trap;

unsafe extern "Rust" {
    /// The main entry point to the kernel.
    safe fn bluemetal(system: &system::System) -> !;
}
unsafe extern "C" {
    /// Halt the hardware thread.
    unsafe fn _hang() -> !;
}

// /// Initialisation from hard coded device configuration.
// ///
// /// Allows unconditionally setting up hardware before dynamic initialisation.
// mod cfg;

#[cfg(feature = "dtb")]
/// Initialisation from a device tree.
mod dtb;

#[cfg(has_system_description)]
pub const SYSTEM: &'static system::System = system::include!(env!("BLUEMETAL_SYSTEM_DESCRIPTION")).expect("invalid system description table");

#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
#[cfg(feature = "dtb")]
#[cfg(not(feature = "uefi"))]
#[unsafe(no_mangle)]
extern "C" fn init(hart_id: usize, dtb: Option<Dtb<'static>>) -> ! {
    // cfg::init();
    let Some(dtb) = dtb.and_then(Dtb::check) else {
        // Not a whole lot can be done if the target hardware is unknown.
        unsafe { _hang() };
    };
    dtb::init(dtb);
    bluemetal(SYSTEM)
}
