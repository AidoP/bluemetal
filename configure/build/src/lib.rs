const PKG_DIR: &str = env!("CARGO_MANIFEST_DIR");

pub struct Profile {
    build: cc::Build,
    profile: configure_options::Profile
}
impl Profile {
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
    pub fn cfg(&self) -> &Self {
        println!("cargo::rustc-cfg=target_machine={:?}", self.profile.machine.cfg());
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
        self
    }
    /// Build the runtime library for `init`.
    pub fn libinit(&self) -> &Self {
        self.library("init", &[
            &format!("src/init/{}", self.profile.entry.file_name()),
            "src/trap.s",
        ]);
        self
    }
}
