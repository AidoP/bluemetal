const PKG_DIR: &str = env!("CARGO_MANIFEST_DIR");

macro_rules! features {
    (
        $(#[doc = $mod_doc:expr])*
        mod $feature:ident {
            $(
                $(#[doc = $feature_doc:expr])*
                $name:ident,
            )*
        }
    ) => {
        mod $feature {
            /// Emit a `rustc-check-cfg` directive to allow all possible values.
            pub fn check() {
                println!(concat!("cargo::rustc-check-cfg=cfg(", stringify!($feature), ", values(", $("\"", stringify!($name), "\"",)* "))"));
            }
            $(
                #[allow(non_upper_case_globals)]
                $(#[doc = $feature_doc])*
                pub const $name: &str = stringify!($name);
            )*
        }
    };
}
features! {
    /// Hardware devices that may be available on the target machine.
    mod target_device {
        /// SiFive-style UART.
        ///
        /// On boards such as:
        /// - [`FU540-C000`](https://sifive.cdn.prismic.io/sifive/d3ed5cd0-6e74-46b2-a12d-72b06706513e_fu540-c000-manual-v1p4.pdf#%5B%7B%22num%22%3A241%2C%22gen%22%3A0%7D%2C%7B%22name%22%3A%22XYZ%22%7D%2C0%2C630%2C0%5D)
        sifive_uart,
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug)]
pub enum Machine {
    sifive_fu540,
}
impl Machine {
    pub fn configure() -> Self {
        const MACHINE: &str = env!("CONFIGURE_MACHINE");
        println!("cargo::rustc-cfg=target_machine={MACHINE:?}");
        let machine = match MACHINE {
            "sifive_fu540" => Machine::sifive_fu540,
            name => panic!("unknown target machine {name:?}"),
        };
        machine
    }
    pub fn bin_args(&self) {
        match self {
            _ => {
                println!("cargo::rerun-if-changed={PKG_DIR}/link/{self:?}.ld");
                println!("cargo::rustc-link-arg-bins=-T{PKG_DIR}/link/{self:?}.ld");
                println!("cargo::rustc-link-arg-bins=--oformat=binary");
            },
        }
    }
    pub fn features(&self) {
        target_device::check();
        let devices: &[&str] = match self {
            Self::sifive_fu540 => &[target_device::sifive_uart],
        };

        for device in devices {
            println!("cargo::rustc-cfg=target_device={device:?}");
        }
    }
    pub fn build_lib(&self, lib: &str, files: &[&str]) {
        let mut build = cc::Build::new();
        match self {
            Self::sifive_fu540 => {
                build.compiler("clang")
                    .flag("-Wno-unused-command-line-argument")
                    .flag("-mabi=lp64d");
            },
        }
        for file in files {
            println!("cargo::rerun-if-changed={file}");
            build.file(file);
        }
        build.compile(lib);
    }
    /// The name of the base entry point file for the machine-specific entry
    /// code.
    pub fn init_name(&self) -> &str {
        match self {
            _ => "riscv_m.s",
        }
    }
}
