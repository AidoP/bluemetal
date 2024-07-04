use std::{os::unix::process::CommandExt, path::{Path, PathBuf}, process::ExitCode};

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

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let (path, profile) = match configure_options::load(&args.profile) {
        Err(error) => {
            eprintln!("failed to load profile {:?}: {error}", args.profile);
            return ExitCode::FAILURE;
        },
        Ok((path, profile)) => (path, profile),
    };

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
    ExitCode::SUCCESS
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
    let args = profile.runner.as_slice();
    let Some(program) = args.get(0) else {
        panic!("no runner provided for this profile");
    };
    let mut command = Command::new(program);
    for arg in &args[1..] {
        if arg == "{{BLUEMETAL_IMAGE}}" {
            command.arg(path);
        } else {
            command.arg(arg);
        }
    }
    command.exec();
}
