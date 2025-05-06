use std::{io::Write, os::unix::ffi::OsStrExt, path::PathBuf, str::FromStr};

pub use configure_options as profile;
pub use profile::Profile;

const PKG_DIR: &str = env!("CARGO_MANIFEST_DIR");

pub struct Config {
    build: cc::Build,
    profile: Profile,
}
impl Config {
    pub fn load() -> Self {
        let mut profile = PathBuf::from_str("../../../").unwrap();
        profile.push(env!("BLUEMETAL_PROFILE"));
        let profile = std::fs::read_to_string(profile).expect("unable to read profile specified by `BLUEMETAL_PROFILE`");
        let mut build = cc::Build::new();
        let profile = configure_options::from_str(&profile)
            .expect("failed to load configuration profile");
        if let Some(compiler) = &profile.compiler {
            build.compiler(&compiler.compiler);
            for flag in &compiler.flags {
                build.flag(flag);
            }
        }

        Self {
            build,
            profile,
        }
    }
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    pub fn cfg(&self) -> &Self {
        println!("cargo::rustc-cfg=target_machine={:?}", self.profile.machine.cfg());
        println!("cargo::rustc-check-cfg=cfg(target_device, values({}))", profile::Device::all().join(", "));
        for device in &self.profile.device {
            println!("cargo::rustc-cfg=target_device={:?}", device.cfg());
        }
        println!("cargo::rustc-check-cfg=cfg(has_system_description)");
        if let Some(system) = &self.profile.system {
            println!("cargo::rustc-cfg=has_system_description");
            let mut path;
            if system.is_relative() {
                path = PathBuf::from_str(env!("CARGO_MANIFEST_DIR")).unwrap();
                path.push("system/");
                path.push(system);
            } else {
                path = system.to_owned();
            }
            print!("cargo::rustc-env=BLUEMETAL_SYSTEM_DESCRIPTION=");
            let path = path.as_os_str().as_bytes();
            assert!(!path.contains(&b'\n'), "invalid path specified for `system-description`");
            std::io::stdout().write_all(path).unwrap();
            println!()
        }
        self
    }
    pub fn bin(&self) -> &Self {
        let linker_script = &self.profile.linker_script;
        println!("cargo::rerun-if-changed={PKG_DIR}/link/{linker_script}");
        println!("cargo::rustc-link-arg-bins=-T{PKG_DIR}/link/{linker_script}");
        self
    }
    pub fn library(&self, name: &str, paths: &[&str]) -> &Self {
        self.build.clone().files(paths).compile(name);
        for path in paths {
            println!("cargo::rerun-if-changed={path:?}");
        }
        self
    }
}
