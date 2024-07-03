use std::{os::unix::process::CommandExt, path::{Path, PathBuf}};

use clap::{Parser, Subcommand};
use configure_options::Profile;

#[derive(Debug, Parser)]
#[command(name = "configure", version, about, long_about = None)]
struct Args {
    profile: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build { },
    Run { },
    CargoRunner {
        path: PathBuf,
    },
}

fn main() {
    let args = Args::parse();
    let (path, profile) = configure_options::load(&args.profile).unwrap();

    match args.command {
        Command::Build {  } => {
            build(&path, &profile);
        },
        Command::Run {  } => {
            run(&path, &profile);
        },
        Command::CargoRunner { path } => {
            cargo_runner(&profile, &path);
        },
    }
}

fn build(path: &Path, profile: &Profile) {
    use std::process::Command;

//build-std = ["core", "compiler_builtins"]
//build-std-features = ["compiler-builtins-mem", "panic-unwind"]
    let mut command = Command::new("cargo");
    command.arg("+nightly")
        .arg("build")
        .arg(format!("--target=configure/build/target/{}", profile.target))
        .arg("-Zbuild-std=core,compiler_builtins")
        .arg("-Zbuild-std-features=compiler-builtins-mem,panic-unwind")
        .arg("--package=bluemetal")
        .env("BLUEMETAL_PROFILE", path);
    println!("command: {command:?}");
    let error = command.exec();
    panic!("failed to run cargo: {error}");
}
fn run(path: &Path, profile: &Profile) {
    use std::process::Command;

//build-std = ["core", "compiler_builtins"]
//build-std-features = ["compiler-builtins-mem", "panic-unwind"]
    let mut command = Command::new("cargo");
    command.arg("+nightly")
        .arg("run")
        .arg(format!("--target=configure/build/target/{}", profile.target))
        .arg("-Zbuild-std=core,compiler_builtins")
        .arg("-Zbuild-std-features=compiler-builtins-mem,panic-unwind")
        .arg("--package=bluemetal")
        .env("BLUEMETAL_PROFILE", path);
    println!("command: {command:?}");
    let error = command.exec();
    panic!("failed to run cargo: {error}");
}
fn cargo_runner(profile: &Profile, path: &Path) {
    use std::process::Command;
    Command::new("qemu-system-riscv64")
        .arg("-s")
        .arg("-machine")
        .arg("sifive_u")
        .arg("-m")
        .arg("128M")
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg("stdio")
        .arg("-bios")
        .arg(path)
        .exec();
}
