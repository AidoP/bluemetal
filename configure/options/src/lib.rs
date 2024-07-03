use std::{fmt::Display, path::PathBuf};

use serde::Deserialize;

#[derive(Debug)]
pub enum Error {
    InvalidPath(PathBuf, std::io::Error),
    Toml(toml::de::Error),
}

pub fn load(profile: &str) -> Result<(PathBuf, Profile), Error> {
    let path = if profile.contains('/') {
        PathBuf::from(profile)
    } else {
        let mut path = PathBuf::from("profile/");
        path.push(profile);
        path.set_extension("toml");
        path
    };
    let raw = std::fs::read_to_string(&path).map_err(|e| Error::InvalidPath(path.clone(), e))?;
    toml::from_str(&raw).map(|profile| (path, profile)).map_err(Error::Toml)
}
pub fn from_str(profile: &str) -> Result<Profile, Error> {
    toml::from_str(&profile).map_err(Error::Toml)
}

#[derive(Debug, Deserialize)]
pub struct Profile {
    pub machine: Machine,
    pub target: Target,
    #[serde(rename = "linker-script")]
    pub linker_script: String,
    pub device: Vec<Device>,
    /// Compiler options for `cc`.
    pub compiler: Option<Compiler>,
    pub entry: Entry,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Machine {
    #[serde(rename = "sifive-fu540")]
    SifiveU540,
    #[serde(rename = "qemu-virt")]
    QemuVirt,
}
impl Machine {
    pub fn cfg(self) -> &'static str {
        match self {
            Self::SifiveU540 => "sifive-fu540",
            Self::QemuVirt => "qemu-virt",
        }
    }
}

#[derive(Debug, Deserialize)]
pub enum Target {
    #[serde(rename = "builtin")]
    Builtin(String),
    #[serde(rename = "riscv64")]
    Riscv64,
}
impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin(name) => write!(f, "{}", name),
            Self::Riscv64 => write!(f, "riscv64gc-unknown-bluemetal-elf.json"),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "name")]
pub enum Device {
    #[serde(rename = "sifive_uart")]
    SifiveUart,
}
impl Device {
    pub fn cfg(&self) -> &str {
        match self {
            Self::SifiveUart => "sifive_uart",
        }
    }
}
#[allow(non_camel_case_types)]
#[derive(Debug, Deserialize)]
#[serde(tag = "name")]
pub enum Entry {
    #[serde(rename = "riscv-machine-mode")]
    RiscvMachineMode,
}
impl Entry {
    pub fn file_name(&self) -> &str {
        match self {
            Self::RiscvMachineMode => "riscv_m.s",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename = "compiler")]
pub struct Compiler {
    pub compiler: PathBuf,
    pub flags: Vec<String>,
}