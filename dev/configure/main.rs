use std::{io::IsTerminal, path::{Path, PathBuf}};

mod database;
mod ninja;
mod project;
mod rust_analyzer;
mod target_spec;

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
    if input == "--test" {
        let names: Vec<_> = args.map(|name| name.to_string_lossy().into_owned()).collect();
        return test(&names);
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
    // the configure tool builds itself, and runs its tests, when its source is part of the project
    let self_hosted = Path::new(CONFIGURE_SOURCE).exists();
    if self_hosted {
        // the root build file, as the crates are not declared by one
        let span = project::Span { file: 0, offset: 0, len: 0 };
        let implicit = |name: &str, path: &str, kind, output: Option<&str>| project::rust::Crate {
            name: name.into(),
            name_span: span,
            path: path.into(),
            path_span: span,
            target: host.clone(),
            target_span: span,
            kind,
            output: output.map(PathBuf::from),
            builtin: false,
            implicit: true,
            default: false,
            test: true,
            uses: Vec::new(),
        };
        let configure = implicit("configure", CONFIGURE_SOURCE, project::rust::Kind::Bin, Some(CONFIGURE));
        project.add_implicit_crate(configure, Vec::new());
        // the integration tests run the configure tool
        let tests = implicit("test_dev_configure", "test/dev/configure/main.rs", project::rust::Kind::Test, None);
        project.add_implicit_crate(tests, vec![CONFIGURE.into()]);
    }
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

    // a builtin target which is overridden keeps its specification
    for target in project.custom_targets.iter().filter(|target| !target.overrides) {
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
    // the flag for compiling for a target, and the specification of a custom target which it depends on
    let custom_targets: Vec<_> = project.custom_targets.iter()
        .filter(|target| !target.overrides)
        .map(|target| target.name.clone())
        .collect();
    let target_flag = |target: &str| -> (String, Option<PathBuf>) {
        if custom_targets.iter().any(|custom| custom == target) {
            let path = target_spec::path(target);
            (format!("--target={} -Zunstable-options", path.display()), Some(path))
        } else {
            (format!("--target={target}"), None)
        }
    };

    let depfile_path = std::path::Path::new("build.ninja.d");
    let mut depfile = match std::fs::File::create(depfile_path) {
        Ok(file) => file,
        Err(error) => return Err(Error::Create(depfile_path.into(), error)),
    };
    if let Err(error) = write_depfile(&mut depfile, &project) {
        return Err(Error::Write(depfile_path.into(), error));
    }

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
        inputs: &[&input],
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
        if custom.test {
            let phony = Path::new("test").join(&custom.name);
            let inputs: Vec<_> = custom.inputs.iter().chain(&custom.program).map(PathBuf::as_path).collect();
            build_ninja.build(&ninja::Build { rule: "phony", outputs: &[&phony], inputs: &inputs, ..Default::default() })?;
            tests.push(phony);
            continue;
        }
        // ninja runs commands with `sh`, which must not split or expand the arguments
        let command: Vec<_> = custom.command.iter().map(|argument| shell_quote(argument)).collect();
        let command = command.join(" ").replace('$', "$$");
        let outputs: Vec<_> = custom.outputs.iter().map(PathBuf::as_path).collect();
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
        if custom.default {
            defaults.push(phony);
        }
    }

    let mut crates: Vec<_> = project.crates.into_values().collect();
    crates.sort_unstable_by(|lhs, rhs| (&lhs.name, &lhs.target).cmp(&(&rhs.name, &rhs.target)));
    for c in &crates {
        // a dependency is a library, so it has an output
        let deps: Vec<_> = externs(c, &crates).map(|(_, d)| (d.name.as_str(), d.output().unwrap())).collect();
        let (target, spec) = target_flag(&c.target);
        let flags = rustc_flags(&target, Path::new(&format!("target/{}", c.target)), &deps);
        let deps: Vec<_> = deps.iter().map(|(_, path)| path.as_path()).chain(spec.as_deref()).collect();
        let crate_type = c.crate_type();
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

    let units: Vec<_> = crates.iter().map(|c| Unit {
        label: format!("{}@{}", c.name, c.target),
        name: c.name.clone(),
        root: c.path.clone(),
        target: c.target.clone(),
        spec: target_flag(&c.target).1,
        // a library is checked without `--test`, as its metadata is used by the crates which depend on it
        test: c.test && c.kind != project::rust::Kind::Lib,
        deps: externs(c, &crates).map(|(index, _)| index).collect(),
        // crates from the toolchain, such as `core`, are not edited
        member: c.implicit || !rust_analyzer::absolute(&c.path).starts_with(&sysroot),
        build_file: project.files[c.name_span.file].path.clone(),
    }).collect();

    // each check is an ordinary edge which saves its diagnostics, so that a flycheck can replay them when it is up to
    // date, as rust-analyzer only shows the diagnostics from the latest flycheck
    for unit in &units {
        let deps: Vec<_> = unit.deps.iter()
            .map(|&index| (units[index].name.as_str(), units[index].check_rmeta()))
            .collect();
        let (target, spec) = target_flag(&unit.target);
        let mut flags = rustc_flags(&target, &unit.check_dir(), &deps);
        if unit.test {
            flags.push_str(" --test");
        }
        let deps: Vec<_> = deps.iter().map(|(_, path)| path.as_path()).chain(spec.as_deref()).collect();
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

    // builds what all of the tests need
    let tests: Vec<_> = tests.iter().map(PathBuf::as_path).collect();
    build_ninja.build(&ninja::Build { rule: "phony", outputs: &[Path::new("test")], inputs: &tests, ..Default::default() })?;

    if !defaults.is_empty() {
        let defaults: Vec<_> = defaults.iter().map(PathBuf::as_path).collect();
        build_ninja.default(&defaults)?;
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
    let flychecks = units.iter().zip(&diagnostics).map(|(unit, diagnostics)| database::Flycheck {
        label: &unit.label,
        diagnostics: diagnostics.iter().map(String::as_str).collect(),
    });
    let tests = project.customs.iter().filter(|custom| custom.test).map(|test| database::Test {
        name: &test.name,
        command: test.command.iter().map(String::as_str).collect(),
    });
    let mut bytes = Vec::new();
    let _ = database::write(&mut bytes, &database::Database { flychecks: flychecks.collect(), tests: tests.collect() });
    let database_path = Path::new(database::PATH);
    let written = std::fs::create_dir_all(database_path.parent().unwrap())
        .and_then(|()| std::fs::write(database_path, bytes));
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
    /// The specification of the target, if it is a custom target.
    pub spec: Option<PathBuf>,
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
    let bytes = match std::fs::read(database::PATH) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read {}: {error}", database::PATH);
            return;
        },
    };
    // the entries borrow from the bytes
    let database = match database::read(&bytes) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("failed to read {}: {error}", database::PATH);
            return;
        },
    };
    let Some(entry) = database.flychecks.iter().find(|entry| entry.label == label) else {
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

/// Run tests, or all of them if no names are given, without evaluating the configuration.
///
/// What the tests need is built by ninja first, then each is run in turn from the project root with its output captured,
/// which is shown if it fails.
fn test(names: &[String]) -> Result<(), Error> {
    let targets: Vec<_> = if names.is_empty() {
        vec!["test".to_string()]
    } else {
        names.iter().map(|name| format!("test/{name}")).collect()
    };
    let mut ninja = std::process::Command::new("ninja");
    ninja.args(&targets);
    let built = match ninja.status() {
        Ok(status) => status.success(),
        Err(error) => return Err(Error::Exec(ninja, error)),
    };
    // read after ninja, which reconfigures if needed, so that it is current
    let bytes = match std::fs::read(database::PATH) {
        Ok(bytes) => bytes,
        Err(error) => return Err(Error::Read(database::PATH.into(), error)),
    };
    let database = match database::read(&bytes) {
        Ok(database) => database,
        Err(error) => return Err(Error::Read(database::PATH.into(), error)),
    };
    // an unknown test also fails the build, as it has no target, but is the more useful error
    let mut tests = Vec::new();
    for name in names {
        match database.tests.iter().find(|test| test.name == name) {
            Some(test) => tests.push(test),
            None => return Err(Error::UnknownTest(name.clone())),
        }
    }
    if !built {
        return Err(Error::TestBuild);
    }
    if names.is_empty() {
        tests = database.tests.iter().collect();
    }

    let mut failures = Vec::new();
    for test in &tests {
        let output = std::process::Command::new(test.command[0])
            .args(&test.command[1..])
            .stdin(std::process::Stdio::null())
            .output();
        let failure = match output {
            Ok(output) if output.status.success() => None,
            Ok(output) => {
                let mut report = format!("{}\n", output.status);
                report.push_str(&String::from_utf8_lossy(&output.stdout));
                report.push_str(&String::from_utf8_lossy(&output.stderr));
                Some(report)
            },
            Err(error) => Some(format!("failed to run {:?}: {error}\n", test.command[0])),
        };
        println!("test {} ... {}", test.name, if failure.is_some() { "FAILED" } else { "ok" });
        if let Some(report) = failure {
            failures.push((test.name, report));
        }
    }
    for (name, report) in &failures {
        println!("\n---- {name} ----\n{}", report.trim_end());
    }
    let result = if failures.is_empty() { "ok" } else { "FAILED" };
    println!("\ntest result: {result}. {} passed; {} failed", tests.len() - failures.len(), failures.len());
    if failures.is_empty() { Ok(()) } else { Err(Error::TestsFailed(failures.len())) }
}

/// Returns the flags for compiling a crate with the flag for its target and its dependencies, as names and paths.
///
/// The outputs for a target share a directory, `search`, for the dependencies of `--extern` crates.
fn rustc_flags(target: &str, search: &Path, deps: &[(&str, PathBuf)]) -> String {
    let mut flags = format!("{target} -L dependency={}", search.display());
    for (name, path) in deps {
        flags.push_str(&format!(" --extern {name}={}", path.display()));
    }
    flags
}

/// The configure tool, which is built by `build.ninja` from its source when that is part of the project.
const CONFIGURE: &str = "target/configure";
const CONFIGURE_SOURCE: &str = "dev/configure/main.rs";

/// Quote a string as a single argument for `sh`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Returns the crates passed to a crate with `--extern`, with their indices in `crates`.
///
/// Builtin crates are passed to every other crate for the same target, but not to each other as that would be a cycle,
/// so a builtin crate must `use` the others it depends on. They are not passed to the implicit crates of the build
/// system either, which use the toolchain's standard library.
fn externs<'a>(
    c: &'a project::rust::Crate,
    crates: &'a [project::rust::Crate],
) -> impl Iterator<Item = (usize, &'a project::rust::Crate)> {
    crates.iter().enumerate().filter(move |(_, d)| {
        let builtin = d.builtin && !c.builtin && !c.implicit;
        d.target == c.target && (builtin || c.uses.iter().any(|(name, _)| *name == d.name))
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
    TestBuild,
    TargetSpec(String, &'static str),
    UnknownTest(String),
    TestsFailed(usize),
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
            Self::TestBuild => write!(f, "failed to build the tests"),
            Self::TargetSpec(name, error) => write!(f, "failed to generate the specification of target {name:?}: {error}"),
            Self::UnknownTest(name) => write!(f, "no test named {name:?}"),
            Self::TestsFailed(1) => write!(f, "1 test failed"),
            Self::TestsFailed(count) => write!(f, "{count} tests failed"),
            Self::Exec(cmd, error) => write!(f, "failed to execute {cmd:?}: {error}"),
        }
    }
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
