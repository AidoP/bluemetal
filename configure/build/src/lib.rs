pub use configure_options as profile;
pub use profile::Profile;

const PKG_DIR: &str = env!("CARGO_MANIFEST_DIR");

pub struct Config {
    build: cc::Build,
    profile: Profile,
}
impl Config {
    pub fn load() -> Self {
        const PROFILE: &str = include_str!(concat!("../../../", env!("BLUEMETAL_PROFILE")));
        let mut build = cc::Build::new();
        let profile = configure_options::from_str(PROFILE)
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
