use std::{ffi::OsString, process::Command};

use crate::{
    JobPool,
    Target,
};

#[derive(Debug)]
pub enum Edition {
    Rust2024,
}
impl Edition {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rust2024 => "2024",
        }
    }
}

#[derive(Debug)]
pub enum CrateType {
    Bin,
    Lib,
}
impl CrateType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Lib => "rlib",
        }
    }
}

#[derive(Debug)]
pub struct Rust {
    /// Path to the crate root module.
    path: &'static str,
    edition: Edition,
    crate_type: CrateType,
}
impl super::Tool for Rust {
    fn build(&self, target: &Target, pool: &mut JobPool) -> Result<(), ()> {
        let path = pool.config.source_root.join(self.path);
        let out_path = pool.config.source_root.join(target.name);
        if let Some(dir) = out_path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        let mut emit = OsString::from("--emit=link=");
        emit.push(&out_path);
        let mut command = Command::new("rustc");
        command.args([
            format!("--edition={}", self.edition.as_str()),
            format!("--crate-type={}", self.crate_type.as_str()),
        ]).arg(&emit).arg(&path);
        pool.run(&mut command);
        Ok(())
    }
}
impl Rust {
    pub const fn new(path: &'static str) -> Self {
        Self {
            path,
            edition: Edition::Rust2024,
            crate_type: CrateType::Lib,
        }
    }
    pub const fn crate_type(mut self, ty: CrateType) -> Self {
        self.crate_type = ty;
        self
    }
}
