//! Generation of the files which build a project: the specifications of its custom targets, `build.ninja` and its
//! depfile, `.rust-project.json` and the database read by `configure --check` and `configure --test`.

use std::path::{Path, PathBuf};

use crate::ninja::{self, NinjaFile};
use crate::project::rust::{Crate, Kind};
use crate::project::{CustomKind, CustomTarget, Project};
use crate::{CONFIGURE, Error, check, database, rust_analyzer, target_spec};

/// Write the files generated from a project, which has been read and checked.
///
/// `input` is the root build file, and `self_hosted` is whether the configure tool is built by the project.
pub fn write(mut project: Project, input: &Path, sysroot: &str, self_hosted: bool) -> Result<(), Error> {
    write_target_specs(&project.custom_targets)?;
    write_depfile(&project)?;

    let mut crates: Vec<_> = std::mem::take(&mut project.crates).into_values().collect();
    crates.sort_unstable_by(|lhs, rhs| (&lhs.name, &lhs.target).cmp(&(&rhs.name, &rhs.target)));
    let units = units(&project, &crates, sysroot);
    write_build_ninja(&project, &crates, &units, input, self_hosted)?;
    write_rust_project(sysroot, &units)?;
    write_database(&project, &units)
}

/// A crate which can be checked by rust-analyzer's flycheck.
pub struct Unit {
    /// The label of the flycheck, which is unique.
    pub label: String,
    pub name: String,
    pub root: PathBuf,
    pub target: String,
    /// The specification of the target, if it is a custom target.
    pub spec: Option<PathBuf>,
    /// Whether the crate is compiled as a test, as the configure tool is.
    pub test: bool,
    pub kind: Kind,
    /// The library or binary built from the crate, if it has one.
    pub output: Option<PathBuf>,
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
    pub fn diagnostics(&self) -> PathBuf {
        self.check_dir().join(format!("{}.json", self.name))
    }
}

/// How rustc is told the target of a crate.
struct TargetFlag {
    /// The `--target` flag, which names a builtin target, or gives the path of the specification of a custom target.
    flag: String,
    /// The specification of a custom target, which the crate depends on.
    spec: Option<PathBuf>,
}
impl TargetFlag {
    fn new(target: &str, custom_targets: &[CustomTarget]) -> Self {
        for custom in custom_targets {
            // a builtin target which is overridden keeps its specification
            if custom.overrides || custom.name != target {
                continue;
            }
            let spec = target_spec::path(target);
            return Self { flag: format!("--target={} -Zunstable-options", spec.display()), spec: Some(spec) };
        }
        Self { flag: format!("--target={target}"), spec: None }
    }
}

/// A crate passed to another with `--extern`.
struct Extern<'a> {
    name: &'a str,
    /// The library, or metadata, of the crate.
    path: PathBuf,
}

/// Write the specification of each custom target.
fn write_target_specs(custom_targets: &[CustomTarget]) -> Result<(), Error> {
    for target in custom_targets {
        // a builtin target which is overridden keeps its specification
        if target.overrides {
            continue;
        }
        let inherited = match &target.inherit {
            Some(base) => Some(target_spec_json(base)?),
            None => None,
        };
        let path = target_spec::path(&target.name);
        let spec = match target_spec::spec(target, inherited.as_deref()) {
            Ok(spec) => spec,
            Err(error) => return Err(Error::TargetSpec(target.name.clone(), error)),
        };
        // every crate for the target depends on its specification, so it is only written when it changes
        if std::fs::read_to_string(&path).is_ok_and(|current| current == spec) {
            continue;
        }
        let written = std::fs::create_dir_all(path.parent().unwrap()).and_then(|()| std::fs::write(&path, spec));
        if let Err(error) = written {
            return Err(Error::Write(path, error));
        }
    }
    Ok(())
}

/// Write the depfile of `build.ninja`, which is regenerated when a file the configuration read changes.
fn write_depfile(project: &Project) -> Result<(), Error> {
    let path = Path::new("build.ninja.d");
    let mut file = match std::fs::File::create(path) {
        Ok(file) => file,
        Err(error) => return Err(Error::Create(path.into(), error)),
    };
    if let Err(error) = write_dependencies(&mut file, project) {
        return Err(Error::Write(path.into(), error));
    }
    Ok(())
}

fn write_dependencies(w: &mut dyn std::io::Write, project: &Project) -> Result<(), std::io::Error> {
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

/// Returns the crates as units for rust-analyzer, in the same order.
fn units(project: &Project, crates: &[Crate], sysroot: &str) -> Vec<Unit> {
    let mut units = Vec::new();
    for c in crates {
        units.push(Unit {
            label: format!("{}@{}", c.name, c.target),
            name: c.name.clone(),
            root: c.path.clone(),
            target: c.target.clone(),
            spec: TargetFlag::new(&c.target, &project.custom_targets).spec,
            // a library is checked without `--test`, as its metadata is used by the crates which depend on it
            test: c.test && !matches!(c.kind, Kind::Lib | Kind::ProcMacro),
            kind: c.kind,
            output: c.output(),
            deps: externs(c, crates),
            // crates from the toolchain, such as `core`, are not edited
            member: c.implicit || !rust_analyzer::absolute(&c.path).starts_with(sysroot),
            build_file: project.files[c.name_span.file].path.clone(),
        });
    }
    units
}

fn write_build_ninja(
    project: &Project,
    crates: &[Crate],
    units: &[Unit],
    input: &Path,
    self_hosted: bool,
) -> Result<(), Error> {
    let mut build_ninja = NinjaFile::new("build.ninja")?;

    // `build.ninja` regenerates itself with the configure tool, which it also builds if it is part of the project, so
    // `bootstrap.ninja` is only needed the first time
    let configure = if self_hosted {
        PathBuf::from(CONFIGURE)
    } else {
        std::env::current_exe().unwrap_or_else(|_| CONFIGURE.into())
    };
    let command = format!("{} $in $out", shell_quote(&configure.display().to_string()).replace('$', "$$"));
    build_ninja.rule("configure", &[
        ("command", &command),
        ("description", "CONFIGURE $out"),
        ("pool", "console"),
        ("deps", "gcc"),
        ("depfile", "$out.d"),
        ("generator", "1"),
    ])?;
    let configure_dependency: Vec<_> = self_hosted.then_some(Path::new(CONFIGURE)).into_iter().collect();
    build_ninja.build(&ninja::Build {
        rule: "configure",
        outputs: &[Path::new("build.ninja")],
        inputs: &[input],
        implicit: &configure_dependency,
        ..Default::default()
    })?;

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

    // the targets built by `ninja` without any, which are all targets if there are none
    let mut defaults: Vec<PathBuf> = Vec::new();

    build_ninja.rule("custom", &[("command", "$command"), ("description", "CUSTOM $name")])?;
    let mut tests = Vec::new();
    for custom in &project.customs {
        // a test is run by `configure --test`, which builds its inputs and program with this target first
        let CustomKind::Command { outputs, default } = &custom.kind else {
            let phony = Path::new("test").join(&custom.name);
            let inputs = custom.inputs(&project.runners);
            let inputs: Vec<_> = inputs.iter().chain(&custom.program).map(PathBuf::as_path).collect();
            build_ninja.build(&ninja::Build { rule: "phony", outputs: &[&phony], inputs: &inputs, ..Default::default() })?;
            tests.push(phony);
            continue;
        };
        // ninja runs commands with `sh`, which must not split or expand the arguments
        let command: Vec<_> = custom.command.iter().map(|argument| shell_quote(argument)).collect();
        let command = command.join(" ").replace('$', "$$");
        let outputs: Vec<_> = outputs.iter().map(PathBuf::as_path).collect();
        let inputs: Vec<_> = custom.inputs.iter().map(PathBuf::as_path).collect();
        let program: Vec<_> = custom.program.iter().map(PathBuf::as_path).collect();
        build_ninja.build(&ninja::Build {
            rule: "custom",
            outputs: &outputs,
            inputs: &inputs,
            implicit: &program,
            variables: &[("command", &command), ("name", &custom.name)],
            ..Default::default()
        })?;
        // so that the outputs can be built by name
        let phony = Path::new("custom").join(&custom.name);
        build_ninja.build(&ninja::Build {
            rule: "phony",
            outputs: &[&phony],
            inputs: &outputs,
            ..Default::default()
        })?;
        if *default {
            defaults.push(phony);
        }
    }

    for c in crates {
        // a dependency is a library, so it has an output
        let mut deps = Vec::new();
        for index in externs(c, crates) {
            let d = &crates[index];
            deps.push(Extern { name: &d.name, path: d.output().unwrap() });
        }
        let target = TargetFlag::new(&c.target, &project.custom_targets);
        let flags = rustc_flags(&target.flag, Path::new(&format!("target/{}", c.target)), c.kind, &deps);
        let deps: Vec<_> = deps.iter().map(|d| d.path.as_path()).chain(target.spec.as_deref()).collect();
        let crate_type = c.kind.crate_type();
        let output = c.output();
        if let Some(output) = &output {
            build_ninja.build(&ninja::Build {
                rule: "rustc",
                outputs: &[output],
                inputs: &[&c.path],
                implicit: &deps,
                variables: &[("crate-name", &c.name), ("crate-type", crate_type), ("flags", &flags)],
                ..Default::default()
            })?;
        }
        if c.test {
            build_ninja.build(&ninja::Build {
                rule: "rustc",
                outputs: &[&c.test_harness()],
                inputs: &[&c.path],
                implicit: &deps,
                variables: &[("crate-name", &c.name), ("crate-type", crate_type), ("flags", &format!("{flags} --test"))],
                ..Default::default()
            })?;
        }
        if c.default && let Some(output) = output {
            defaults.push(output);
        }
    }

    // each check is an ordinary edge which saves its diagnostics, so that a flycheck can replay them when it is up to
    // date, as rust-analyzer only shows the diagnostics from the latest flycheck
    for unit in units {
        // a procedural macro is run by rustc, even when checking, so its own output is used rather than its metadata
        let mut deps = Vec::new();
        for &index in &unit.deps {
            let dep = &units[index];
            let path = match dep.kind {
                Kind::ProcMacro => dep.output.clone().unwrap(),
                _ => dep.check_rmeta(),
            };
            deps.push(Extern { name: &dep.name, path });
        }
        let target = TargetFlag::new(&unit.target, &project.custom_targets);
        let mut flags = rustc_flags(&target.flag, &unit.check_dir(), unit.kind, &deps);
        if unit.test {
            flags.push_str(" --test");
        }
        let deps: Vec<_> = deps.iter().map(|d| d.path.as_path()).chain(target.spec.as_deref()).collect();
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
                ("crate-type", unit.kind.crate_type()),
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

    // builds what all of the tests need
    let tests: Vec<_> = tests.iter().map(PathBuf::as_path).collect();
    build_ninja.build(&ninja::Build { rule: "phony", outputs: &[Path::new("test")], inputs: &tests, ..Default::default() })?;

    if !defaults.is_empty() {
        let defaults: Vec<_> = defaults.iter().map(PathBuf::as_path).collect();
        build_ninja.default(&defaults)?;
    }

    build_ninja.sync_all()
}

fn write_rust_project(sysroot: &str, units: &[Unit]) -> Result<(), Error> {
    let path = Path::new(".rust-project.json");
    let mut rust_project = Vec::new();
    let configure = std::env::current_exe().unwrap_or_else(|_| CONFIGURE.into());
    // writing to a buffer cannot fail
    let _ = rust_analyzer::write(&mut rust_project, Path::new(sysroot), &configure, units);
    if let Err(error) = std::fs::write(path, rust_project) {
        return Err(Error::Write(path.into(), error));
    }
    Ok(())
}

/// The command of a test, which the database borrows.
struct TestCommand<'a> {
    name: &'a str,
    command: Vec<String>,
}

fn write_database(project: &Project, units: &[Unit]) -> Result<(), Error> {
    // the database borrows the paths and commands
    let mut replayed = Vec::new();
    for unit in units {
        let mut paths = Vec::new();
        for path in check::replayed(unit, units) {
            paths.push(path.display().to_string());
        }
        replayed.push(paths);
    }
    let mut commands = Vec::new();
    for custom in &project.customs {
        if custom.is_test() {
            commands.push(TestCommand { name: &custom.name, command: custom.command(&project.runners) });
        }
    }

    let mut database = database::Database::default();
    for (unit, paths) in units.iter().zip(&replayed) {
        let diagnostics = paths.iter().map(String::as_str).collect();
        database.flychecks.push(database::Flycheck { label: &unit.label, diagnostics });
    }
    for test in &commands {
        let command = test.command.iter().map(String::as_str).collect();
        database.tests.push(database::Test { name: test.name, command });
    }
    let mut bytes = Vec::new();
    let _ = database::write(&mut bytes, &database);
    let path = Path::new(database::PATH);
    let written = std::fs::create_dir_all(path.parent().unwrap()).and_then(|()| std::fs::write(path, bytes));
    if let Err(error) = written {
        return Err(Error::Write(path.into(), error));
    }
    Ok(())
}

/// Returns the flags for compiling a crate of a kind with the flag for its target and the crates passed to it.
///
/// The outputs for a target share a directory, `search`, for the dependencies of `--extern` crates.
fn rustc_flags(target: &str, search: &Path, kind: Kind, deps: &[Extern]) -> String {
    let mut flags = format!("{target} -L dependency={}", search.display());
    // the toolchain's `proc_macro` crate is not in the extern prelude otherwise
    if kind == Kind::ProcMacro {
        flags.push_str(" --extern proc_macro");
    }
    for dep in deps {
        flags.push_str(&format!(" --extern {}={}", dep.name, dep.path.display()));
    }
    flags
}

/// Quote a string as a single argument for `sh`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Returns the indices in `crates` of the crates passed to a crate with `--extern`.
///
/// Builtin crates are passed to every other crate for the same target, but not to each other as that would be a cycle,
/// so a builtin crate must `use` the others it depends on. They are not passed to the implicit crates of the build
/// system either, which use the toolchain's standard library.
fn externs(c: &Crate, crates: &[Crate]) -> Vec<usize> {
    let mut externs = Vec::new();
    for (index, d) in crates.iter().enumerate() {
        let builtin = d.target == c.target && d.builtin && !c.builtin && !c.implicit;
        // a procedural macro is for the host target, but can be used by a crate for any target
        let target = d.target == c.target || d.kind == Kind::ProcMacro;
        if builtin || target && d.kind != Kind::Bin && c.uses.iter().any(|used| used.name == d.name) {
            externs.push(index);
        }
    }
    externs
}

/// Returns the specification of a builtin target, which is unstable.
fn target_spec_json(target: &str) -> Result<String, Error> {
    let mut cmd = std::process::Command::new("rustc");
    cmd.args(["-Zunstable-options", "--print=target-spec-json", &format!("--target={target}")]);
    let output = match cmd.output() {
        Ok(output) => output,
        Err(error) => return Err(Error::Exec(cmd, error)),
    };
    match String::from_utf8(output.stdout) {
        Ok(json) if output.status.success() => Ok(json),
        _ => Err(Error::InvalidOutput(cmd)),
    }
}
