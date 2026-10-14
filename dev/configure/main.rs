use std::{io::IsTerminal, path::{Path, PathBuf}};

mod flycheck;
mod ninja;
mod project;
mod rust_analyzer;

use ninja::NinjaFile;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or("configure".into());
    match run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: {error}", program.display());
            std::process::ExitCode::FAILURE
        },
    }
}

fn run(mut args: std::env::ArgsOs) -> Result<(), Error> {
    let Some(input) = args.next() else {
        return Err(Error::MissingArg("input"));
    };
    if input == "--check" {
        let Some(label) = args.next() else {
            return Err(Error::MissingArg("label"));
        };
        check(&label.to_string_lossy());
        return Ok(());
    }
    let input = PathBuf::from(input);

    let sysroot = rustc_print("sysroot")?;
    let host = rustc_print("host-tuple")?;
    let targets = rustc_print("target-list")?.lines().map(String::from).collect();

    let mut project = project::Project::new(input.parent().map_or_default(|path| path.into()), targets);
    let file = match project::File::read(input.clone()) {
        Ok(file) => file,
        Err(error) => return Err(Error::Read(input, error)),
    };
    let builtins = [
        (
            project::Property::new("sysroot".into(), project::Type::String)
                .info("the sysroot of the Rust toolchain"),
            sysroot.clone(),
        ),
        (
            project::Property::new("target".into(), project::Type::String)
                .info("the target triple for crates without a `target` attribute"),
            host.clone(),
        ),
        (
            project::Property::new("host-target".into(), project::Type::String)
                .info("the target triple of the machine running the build")
                .read_only(),
            host.clone(),
        ),
    ];
    for (property, value) in builtins {
        let name = property.name().to_string();
        let id = project.declare(name.clone(), Some(property));
        project.set(name, project::Value::new(id, project::Data::Str(value)));
    }
    project.parse(file);
    project.check();

    if !project.diagnostics.is_empty() {
        let mut errors = 0;
        let mut warnings = 0;
        let mut stderr = std::io::stderr().lock();
        let pretty = stderr.is_terminal();
        for diagnostic in &project.diagnostics {
            match diagnostic.severity {
                project::Severity::Error => errors += 1,
                project::Severity::Warning => warnings += 1,
                _ => (),
            }
            if let Err(error) = project.write_diagnostic(&mut stderr, diagnostic, pretty) {
                return Err(Error::Stderr(error));
            }
        }
        let result = Error::Diagnostics { errors, warnings };
        if errors > 0 {
            return Err(result);
        } else if warnings > 0 {
            let diagnostic = project::Diagnostic {
                span: None,
                message: project::Message::User(result.to_string()),
                severity: project::Severity::Warning,
            };
            if let Err(error) = project.write_diagnostic(&mut stderr, &diagnostic, pretty) {
                return Err(Error::Stderr(error));
            }
        }
    }

    let depfile_path = std::path::Path::new("build.ninja.d");
    let mut depfile = match std::fs::File::create(depfile_path) {
        Ok(file) => file,
        Err(error) => return Err(Error::Create(depfile_path.into(), error)),
    };
    if let Err(error) = write_depfile(&mut depfile, &project) {
        return Err(Error::Write(depfile_path.into(), error));
    }

    let mut build_ninja = NinjaFile::new("build.ninja")?;
    build_ninja.subninja("bootstrap.ninja")?;

    build_ninja.variable("edition", "2024")?;
    build_ninja.rule("rustc", &[
        ("command", "rustc --color=always --out-dir=target/tmp --emit=link=$out --emit=dep-info=$depfile --crate-type=$crate-type --crate-name=$crate-name --edition=$edition $flags $in"),
        ("description", "RUSTC $out"),
        ("deps", "gcc"),
        ("depfile", "$out.d"),
    ])?;

    build_ninja.rule("rustc-check", &[
        ("command", "rustc --error-format=json --out-dir=$outdir --emit=metadata=$out --emit=dep-info=$out.d --crate-type=$crate-type --crate-name=$crate-name --edition=$edition $flags $in 2> $diagnostics"),
        ("description", "CHECK $out"),
        ("deps", "gcc"),
        ("depfile", "$out.d"),
    ])?;

    let mut crates: Vec<_> = project.crates.into_values().collect();
    crates.sort_unstable_by(|lhs, rhs| (&lhs.name, &lhs.target).cmp(&(&rhs.name, &rhs.target)));
    let rlib = |c: &project::rust::Crate| PathBuf::from(format!("target/{}/lib{}.rlib", c.target, c.name));
    for c in &crates {
        let output = rlib(c);
        let deps: Vec<_> = externs(c, &crates).map(|(_, d)| (d.name.as_str(), rlib(d))).collect();
        let flags = rustc_flags(&c.target, Path::new(&format!("target/{}", c.target)), &deps);
        let deps: Vec<_> = deps.iter().map(|(_, path)| path.as_path()).collect();
        build_ninja.build(&ninja::Build {
            rule: "rustc",
            outputs: &[&output],
            inputs: &[&c.path],
            implicit: &deps,
            variables: &[("crate-name", &c.name), ("crate-type", "lib"), ("flags", &flags)],
            ..Default::default()
        })?;
    }

    let mut units: Vec<_> = crates.iter().map(|c| Unit {
        label: format!("{}@{}", c.name, c.target),
        name: c.name.clone(),
        root: c.path.clone(),
        target: c.target.clone(),
        test: false,
        deps: externs(c, &crates).map(|(index, _)| index).collect(),
        // crates from the toolchain, such as `core`, are not edited
        member: !rust_analyzer::absolute(&c.path).starts_with(&sysroot),
        build_file: project.files[c.name_span.file].path.clone(),
    }).collect();
    // the configure tool is not declared by the configuration, but is checked like the crates which are
    for (name, root) in [("configure", "dev/configure/main.rs"), ("test_dev_configure", "test/dev/configure/main.rs")] {
        units.push(Unit {
            label: name.into(),
            name: name.into(),
            root: root.into(),
            target: host.clone(),
            test: true,
            deps: Vec::new(),
            member: true,
            build_file: "bootstrap.ninja".into(),
        });
    }

    // each check is an ordinary edge which saves its diagnostics, so that a flycheck can replay them when it is up to
    // date, as rust-analyzer only shows the diagnostics from the latest flycheck
    for unit in &units {
        let deps: Vec<_> = unit.deps.iter()
            .map(|&index| (units[index].name.as_str(), units[index].check_rmeta()))
            .collect();
        let mut flags = rustc_flags(&unit.target, &unit.check_dir(), &deps);
        if unit.test {
            flags.push_str(" --test");
        }
        let deps: Vec<_> = deps.iter().map(|(_, path)| path.as_path()).collect();
        let output = unit.check_rmeta();
        let diagnostics = unit.diagnostics();
        let outdir = unit.check_dir();
        build_ninja.build(&ninja::Build {
            rule: "rustc-check",
            outputs: &[&output],
            implicit_outputs: &[&diagnostics],
            inputs: &[&unit.root],
            implicit: &deps,
            variables: &[
                ("crate-name", &unit.name),
                ("crate-type", if unit.test { "bin" } else { "lib" }),
                ("flags", &flags),
                ("outdir", &outdir.display().to_string()),
                ("diagnostics", &diagnostics.display().to_string()),
            ],
            ..Default::default()
        })?;
        build_ninja.build(&ninja::Build {
            rule: "phony",
            outputs: &[&Path::new("check").join(&unit.label)],
            inputs: &[&output],
            ..Default::default()
        })?;
    }

    build_ninja.sync_all()?;

    let rust_project_path = Path::new(".rust-project.json");
    let mut rust_project = Vec::new();
    let configure = std::env::current_exe().unwrap_or_else(|_| "target/configure".into());
    // writing to a buffer cannot fail
    let _ = rust_analyzer::write(&mut rust_project, Path::new(&sysroot), &configure, &units);
    if let Err(error) = std::fs::write(rust_project_path, rust_project) {
        return Err(Error::Write(rust_project_path.into(), error));
    }

    let diagnostics: Vec<Vec<_>> = units.iter()
        .map(|unit| replayed(unit, &units).iter().map(|path| path.display().to_string()).collect())
        .collect();
    let entries: Vec<_> = units.iter().zip(&diagnostics).map(|(unit, diagnostics)| flycheck::Entry {
        label: &unit.label,
        diagnostics: diagnostics.iter().map(String::as_str).collect(),
    }).collect();
    let mut database = Vec::new();
    let _ = flycheck::write(&mut database, &entries);
    let database_path = Path::new(flycheck::PATH);
    let written = std::fs::create_dir_all(database_path.parent().unwrap())
        .and_then(|()| std::fs::write(database_path, database));
    if let Err(error) = written {
        return Err(Error::Write(database_path.into(), error));
    }

    Ok(())
}

/// A crate which can be checked by rust-analyzer's flycheck.
pub struct Unit {
    /// The label of the flycheck, which is unique.
    pub label: String,
    pub name: String,
    pub root: PathBuf,
    pub target: String,
    /// Whether the crate is compiled as a test, as the configure tool is.
    pub test: bool,
    /// The indices of the crates passed with `--extern`.
    pub deps: Vec<usize>,
    /// Whether the crate is edited, rather than part of the toolchain.
    pub member: bool,
    /// The file which declares the crate.
    pub build_file: PathBuf,
}
impl Unit {
    fn check_dir(&self) -> PathBuf {
        PathBuf::from(format!("target/{}/check", self.target))
    }
    fn check_rmeta(&self) -> PathBuf {
        self.check_dir().join(format!("lib{}.rmeta", self.name))
    }
    /// The file of JSON diagnostics from checking the crate.
    fn diagnostics(&self) -> PathBuf {
        self.check_dir().join(format!("{}.json", self.name))
    }
}

/// Returns the diagnostics to replay for a flycheck of a unit: its own, then those of the units it depends on, directly
/// or not, which are edited.
fn replayed(unit: &Unit, units: &[Unit]) -> Vec<PathBuf> {
    let mut seen = vec![false; units.len()];
    let mut stack: Vec<_> = unit.deps.iter().rev().copied().collect();
    let mut paths = vec![unit.diagnostics()];
    while let Some(index) = stack.pop() {
        if std::mem::replace(&mut seen[index], true) {
            continue;
        }
        if units[index].member {
            paths.push(units[index].diagnostics());
        }
        stack.extend(units[index].deps.iter().rev());
    }
    paths
}

/// Run a flycheck for rust-analyzer, without evaluating the configuration.
///
/// The checks are brought up to date by ninja, then their saved diagnostics are copied to stdout, so that diagnostics
/// are shown even when nothing needed checking. Problems are reported on stderr, which rust-analyzer ignores, as is the
/// exit status.
fn check(label: &str) {
    // ninja's output is not JSON, so it is kept out of stdout
    let status = std::process::Command::new("ninja")
        .args(["--quiet", &format!("check/{label}")])
        .stdout(std::io::stderr())
        .status();
    if let Err(error) = status {
        eprintln!("failed to run ninja: {error}");
    }
    // read after ninja, which reconfigures if needed, so that it is current
    let bytes = match std::fs::read(flycheck::PATH) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read {}: {error}", flycheck::PATH);
            return;
        },
    };
    // the entries borrow from the bytes
    let entries = match flycheck::read(&bytes) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("failed to read {}: {error}", flycheck::PATH);
            return;
        },
    };
    let Some(entry) = entries.iter().find(|entry| entry.label == label) else {
        eprintln!("no crate with the label {label:?}");
        return;
    };
    let mut stdout = std::io::stdout().lock();
    for path in &entry.diagnostics {
        // a check which has not run, as a dependency failed, has no diagnostics
        if let Ok(mut file) = std::fs::File::open(path) {
            let _ = std::io::copy(&mut file, &mut stdout);
        }
    }
}

/// Returns the flags for compiling a crate for a target with dependencies, as names and paths.
///
/// The outputs for a target share a directory, `search`, for the dependencies of `--extern` crates.
fn rustc_flags(target: &str, search: &Path, deps: &[(&str, PathBuf)]) -> String {
    let mut flags = format!("--target={target} -L dependency={}", search.display());
    for (name, path) in deps {
        flags.push_str(&format!(" --extern {name}={}", path.display()));
    }
    flags
}

/// Returns the crates passed to a crate with `--extern`, with their indices in `crates`.
///
/// Builtin crates are passed to every other crate for the same target, but not to each other as that would be a cycle,
/// so a builtin crate must `use` the others it depends on.
fn externs<'a>(
    c: &'a project::rust::Crate,
    crates: &'a [project::rust::Crate],
) -> impl Iterator<Item = (usize, &'a project::rust::Crate)> {
    crates.iter().enumerate().filter(move |(_, d)| {
        d.target == c.target && ((d.builtin && !c.builtin) || c.uses.iter().any(|(name, _)| *name == d.name))
    })
}

fn write_depfile(w: &mut dyn std::io::Write, project: &project::Project) -> Result<(), std::io::Error> {
    write!(w, "build.ninja:")?;
    let paths = project.files.iter().map(|file| &file.path).chain(&project.dependencies);
    for path in paths {
        // escape the characters which are special in a depfile
        let path = path.display().to_string().replace(' ', "\\ ").replace('#', "\\#").replace('$', "$$");
        write!(w, " {path}")?;
    }
    writeln!(w)?;
    Ok(())
}

enum Error {
    MissingArg(&'static str),
    Read(std::path::PathBuf, std::io::Error),
    Create(std::path::PathBuf, std::io::Error),
    Write(std::path::PathBuf, std::io::Error),
    Diagnostics {
        errors: usize,
        warnings: usize,
    },
    Stderr(std::io::Error),
    InvalidOutput(std::process::Command),
    Exec(std::process::Command, std::io::Error),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::MissingArg(name) => write!(f, "missing required argument `{name}`"),
            Self::Read(path, error) => write!(f, "unable to read {path:?}: {error}"),
            Self::Create(path, error) => write!(f, "failed to create {path:?}: {error}"),
            Self::Write(path, error) => write!(f, "failed to write to {path:?}: {error}"),
            Self::Diagnostics { errors: 0, warnings: 1 } => write!(f, "configuration raised 1 warning"),
            Self::Diagnostics { errors: 1, warnings: 0 } => write!(f, "configuration failed with 1 error"),
            Self::Diagnostics { errors: 0, warnings } => write!(f, "configuration raised {warnings} warnings"),
            Self::Diagnostics { errors, warnings: 0 } => write!(f, "configuration failed with {errors} errors"),
            Self::Diagnostics { errors, warnings } => {
                let error_suffix = if *errors > 1 { "s" } else { "" };
                let warning_suffix = if *warnings > 1 { "s" } else { "" };
                write!(f, "configuration failed with {errors} error{error_suffix}, {warnings} warning{warning_suffix}")
            },
            Self::Stderr(error) => write!(f, "failed to write to stderr: {error}"),
            Self::InvalidOutput(cmd) => write!(f, "invalid output from command {cmd:?}"),
            Self::Exec(cmd, error) => write!(f, "failed to execute {cmd:?}: {error}"),
        }
    }
}

/// Returns the trimmed output of `rustc --print=<request>`.
fn rustc_print(request: &str) -> Result<String, Error> {
    use std::process::Command;
    let mut cmd = Command::new("rustc");
    cmd.arg(format!("--print={request}"));
    let output = match cmd.output() {
        Err(error) => {
            return Err(Error::Exec(cmd, error));
        },
        Ok(output) => output,
    };
    if !output.status.success() {
        return Err(Error::InvalidOutput(cmd));
    }
    match String::from_utf8(output.stdout) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().into()),
        _ => Err(Error::InvalidOutput(cmd)),
    }
}
