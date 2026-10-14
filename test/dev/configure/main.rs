//! Integration tests for `target/configure`.
//!
//! Each test writes a project to a temporary directory, runs `configure` on its `Build` file from that directory, and
//! checks the diagnostics and the generated `build.ninja`.
//!
//! The `configure` binary is found with the `CONFIGURE` environment variable, or at `target/configure` relative to the
//! working directory.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A project in a temporary directory, removed when dropped.
struct Project {
    dir: PathBuf,
}
impl Project {
    /// Create a project from `(path, contents)` pairs. The first file should be the root `Build`.
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("bluemetal-configure-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, contents) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        Self { dir }
    }
    fn configure(&self) -> Output {
        let configure = configure_path();
        let output = Command::new(&configure)
            .arg("Build")
            .current_dir(&self.dir)
            .output()
            .unwrap_or_else(|error| panic!("failed to run {configure:?}: {error}"));
        Output {
            success: output.status.success(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            ninja: std::fs::read_to_string(self.dir.join("build.ninja")).unwrap_or_default(),
        }
    }
    /// Write a `bootstrap.ninja` which regenerates `build.ninja`, which `build.ninja` includes, so that ninja can be run.
    fn bootstrap(&self) {
        let bootstrap = format!(
            "rule configure\n    command = {} $in $out\n    deps = gcc\n    depfile = $out.d\nbuild build.ninja: configure Build\n",
            configure_path().display(),
        );
        std::fs::write(self.dir.join("bootstrap.ninja"), bootstrap).unwrap();
    }
    /// Run `configure --check` for a label, returning stdout and stderr.
    #[track_caller]
    fn check(&self, label: &str) -> (String, String) {
        let output = Command::new(configure_path()).args(["--check", label]).current_dir(&self.dir).output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        // a failed check is still a successful flycheck
        assert!(output.status.success(), "--check failed:\n{stdout}\n{stderr}");
        (stdout, stderr)
    }
    /// The host triple, as seen by `rustc` from the project directory.
    fn host(&self) -> String {
        let output = Command::new("rustc").arg("--print=host-tuple").current_dir(&self.dir).output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
}
fn configure_path() -> PathBuf {
    match std::env::var_os("CONFIGURE") {
        Some(path) => std::path::absolute(path).unwrap(),
        None => std::env::current_dir().unwrap().join("target/configure"),
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Output {
    success: bool,
    stderr: String,
    ninja: String,
}
impl Output {
    /// Assert that configuration succeeded, returning the output for further checks.
    #[track_caller]
    fn ok(self) -> Self {
        assert!(self.success, "configure failed:\n{}", self.stderr);
        self
    }
    /// Assert that configuration failed, returning the output for further checks.
    #[track_caller]
    fn failed(self) -> Self {
        assert!(!self.success, "configure succeeded:\n{}", self.stderr);
        self
    }
    /// Assert that stderr contains `expected`.
    #[track_caller]
    fn stderr(&self, expected: &str) -> &Self {
        assert!(self.stderr.contains(expected), "stderr does not contain {expected:?}:\n{}", self.stderr);
        self
    }
    /// Returns the `build` statement for an output, including its variables.
    #[track_caller]
    fn build(&self, output: &str) -> &str {
        let prefix = format!("build {output}:");
        let Some(start) = self.ninja.find(&prefix) else {
            panic!("no build statement for {output:?}:\n{}", self.ninja);
        };
        let rest = &self.ninja[start..];
        &rest[..rest.find("\n\n").unwrap_or(rest.len())]
    }
    /// Returns the outputs of the crates' `build` statements, rather than their checks.
    fn outputs(&self) -> Vec<&str> {
        let outputs = self.ninja.lines().filter_map(|line| line.strip_prefix("build ")?.split(':').next());
        outputs.filter(|output| output.ends_with(".rlib")).collect()
    }
}

#[track_caller]
fn assert_contains(haystack: &str, needle: &str) {
    assert!(haystack.contains(needle), "{needle:?} not found in:\n{haystack}");
}

#[test]
fn log_evaluates_expressions() {
    let project = Project::new("log", &[("Build", "\
declare pages
    type 'unsigned'
    default 4
log note 'size {pages * 0x1000} {{literal}}'
log warning 'careful'
")]);
    project.configure().ok().stderr("note: size 16384 {literal}").stderr("warning: careful");
}

#[test]
fn target_defaults_to_host() {
    let project = Project::new("target-default", &[("Build", "crate app 'app.rs'\n")]);
    let host = project.host();
    let output = project.configure().ok();
    let build = output.build(&format!("target/{host}/libapp.rlib"));
    assert_contains(build, "crate-name = app");
    assert_contains(build, &format!("--target={host}"));
    assert_contains(build, &format!("-L dependency=target/{host}"));
}

#[test]
fn target_is_resolved_at_declaration() {
    let project = Project::new("target-resolve", &[("Build", "\
crate before 'x.rs'
set target 'riscv64gc-unknown-none-elf'
crate after 'x.rs'
crate attribute 'x.rs'
    target 'aarch64-unknown-none'
crate host 'x.rs'
    target host-target
log note 'host-target is {host-target}'
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr(&format!("note: host-target is {host}"));
    output.build(&format!("target/{host}/libbefore.rlib"));
    output.build("target/riscv64gc-unknown-none-elf/libafter.rlib");
    output.build("target/aarch64-unknown-none/libattribute.rlib");
    output.build(&format!("target/{host}/libhost.rlib"));
}

#[test]
fn crate_default_path() {
    let project = Project::new("crate-path", &[
        ("Build", "use 'lib'\n"),
        ("lib/Build", "declare suffix\n    type 'str'\n    default '2'\ncrate {'util{suffix}'}\n"),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    assert_contains(output.build(&format!("target/{host}/libutil2.rlib")), ": rustc lib/util2/lib.rs");
}

#[test]
fn crates_are_keyed_by_target() {
    let project = Project::new("crate-key", &[("Build", "\
crate core 'core.rs'
crate core 'core.rs'
    target 'riscv64gc-unknown-none-elf'
")]);
    let host = project.host();
    let output = project.configure().ok();
    let mut outputs = output.outputs();
    outputs.sort();
    assert_eq!(outputs, [
        "target/riscv64gc-unknown-none-elf/libcore.rlib".to_string(),
        format!("target/{host}/libcore.rlib"),
    ]);
}

#[test]
fn crate_redeclaration() {
    let project = Project::new("crate-redeclare", &[("Build", "\
crate dup 'x.rs'
crate dup 'y.rs'
")]);
    project.configure().failed()
        .stderr("error: redeclaration of crate \"dup\"")
        .stderr(" --> Build:2:7")
        .stderr("note: previously declared here\n --> Build:1:7");
}

#[test]
fn builtin_crates_are_extern_by_default() {
    let project = Project::new("builtin", &[("Build", "\
crate core 'core.rs'
    builtin
crate compiler_builtins 'cb.rs'
    builtin
    use core
crate app 'app.rs'
crate kernel 'kernel.rs'
    target 'riscv64gc-unknown-none-elf'
")]);
    let host = project.host();
    let output = project.configure().ok();
    let core = format!("target/{host}/libcore.rlib");
    let compiler_builtins = format!("target/{host}/libcompiler_builtins.rlib");

    let app = output.build(&format!("target/{host}/libapp.rlib"));
    assert_contains(app, &format!("| {compiler_builtins} {core}\n"));
    assert_contains(app, &format!("--extern compiler_builtins={compiler_builtins} --extern core={core}"));

    // builtins only receive the crates they use
    let core_build = output.build(&core);
    assert!(!core_build.contains("--extern"), "{core_build}");
    assert_contains(output.build(&compiler_builtins), &format!("--extern core={core}"));

    // builtins are only passed to crates for the same target
    let kernel = output.build("target/riscv64gc-unknown-none-elf/libkernel.rlib");
    assert!(!kernel.contains("--extern"), "{kernel}");
}

#[test]
fn use_adds_dependency() {
    let project = Project::new("use", &[
        ("Build", "declare name\n    default 'util'\ncrate app 'app.rs'\n    use {name}\nuse 'util'\n"),
        ("util/Build", "crate util\n"),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    let util = format!("target/{host}/libutil.rlib");
    let app = output.build(&format!("target/{host}/libapp.rlib"));
    assert_contains(app, &format!("| {util}\n"));
    assert_contains(app, &format!("--extern util={util}"));
}

#[test]
fn use_unknown_crate() {
    let project = Project::new("use-unknown", &[("Build", "\
crate util 'u.rs'
crate kernel 'k.rs'
    target 'riscv64gc-unknown-none-elf'
    use util
crate bad 'b.rs'
    use 5
    use 'quoted'
    use
    use {'a b'}
")]);
    project.configure().failed()
        .stderr("error: no crate \"util\" for target \"riscv64gc-unknown-none-elf\"\n --> Build:4:9")
        .stderr("error: invalid character '5' in identifier \"5\"\n --> Build:6:9")
        .stderr("error: expected identifier\n --> Build:7:9")
        .stderr("error: expected \" \"\n --> Build:8:8")
        .stderr("error: invalid character ' ' in identifier \"a b\"\n --> Build:9:9")
        .stderr("configuration failed with 5 errors");
}

#[test]
fn unknown_target() {
    let project = Project::new("target-unknown", &[("Build", "\
crate a 'x.rs'
    target 'riscv64gc-unknown-none'
crate b 'x.rs'
    target 'nope'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: unknown target \"riscv64gc-unknown-none\", did you mean \"riscv64gc-unknown-none-elf\"?\n --> Build:2:12")
        .stderr("error: unknown target \"nope\"\n --> Build:4:12")
        .stderr("note: available targets are:\n      aarch64-")
        .stderr("\n      riscv64gc-unknown-none-elf\n");
    assert_eq!(output.stderr.matches("available targets are").count(), 1, "{}", output.stderr);
}

#[test]
fn invalid_crate_name() {
    let project = Project::new("crate-name", &[("Build", "\
declare suffix
    type 'str'
    default 'x'
crate host-tool 'x.rs'
crate {'a-{suffix}'} 'x.rs'
crate café 'x.rs'
crate 2fast 'x.rs'
crate 'quoted' 'x.rs'
crate {'a b'} 'x.rs'
crate {''} 'x.rs'
crate compiler_builtins2 'x.rs'
crate Upper 'x.rs'
")]);
    project.configure().failed()
        // an identifier points at the character, an expression at the whole name
        .stderr("error: invalid character '-' in crate name \"host-tool\"\n --> Build:4:11")
        .stderr("error: invalid character '-' in crate name \"a-x\"\n --> Build:5:7")
        .stderr("error: invalid character 'é' in identifier \"café\"\n --> Build:6:10")
        .stderr("error: invalid character '2' in identifier \"2fast\"\n --> Build:7:7")
        .stderr("error: expected identifier\n --> Build:8:7")
        .stderr("error: invalid character ' ' in identifier \"a b\"\n --> Build:9:7")
        .stderr("error: expected identifier\n  --> Build:10:7")
        .stderr("configuration failed with 7 errors");
}

#[test]
fn invalid_identifier() {
    let project = Project::new("ident", &[("Build", "\
declare my.prop
declare prop2
    type 'str'
    default 'v'
    défaut 'x'
Sét prop2 'y'
log note '{Prøp}'
declare 2go
set {5} 1
")]);
    project.configure().failed()
        .stderr("error: expected \"\\n\"\n --> Build:1:11")
        .stderr("error: invalid character 'é' in identifier \"défaut\"\n --> Build:5:6")
        .stderr("error: invalid character 'é' in identifier \"Sét\"\n --> Build:6:2")
        .stderr("error: invalid character 'ø' in identifier \"Prøp\"\n --> Build:7:14")
        .stderr("error: invalid character '2' in identifier \"2go\"\n --> Build:8:9")
        .stderr("error: invalid character '5' in identifier \"5\"\n --> Build:9:5")
        .stderr("configuration failed with 6 errors");
}

#[test]
fn computed_names() {
    let project = Project::new("names", &[("Build", "\
declare my_prop
    default 'core'
declare suffix
    default 2
declare other_2
set {'other_{suffix}'} 1
log note 'other_2 is {other_2}'
crate compiler_builtins 'x.rs'
crate {'{my_prop}_ext'} 'y.rs'
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr("note: other_2 is 1");
    output.build(&format!("target/{host}/libcompiler_builtins.rlib"));
    output.build(&format!("target/{host}/libcore_ext.rlib"));
}

#[test]
fn writes_depfile() {
    let project = Project::new("depfile", &[
        ("Build", "use 'a'\n"),
        ("a/Build", "crate a 'a.rs'\n"),
    ]);
    project.configure().ok();
    let depfile = std::fs::read_to_string(Path::new(&project.dir).join("build.ninja.d")).unwrap();
    assert_eq!(depfile, "build.ninja: Build a/Build\n");
}

#[test]
fn read_only_property() {
    let project = Project::new("read-only", &[("Build", "\
declare version
    type 'unsigned'
    default 3
    read-only
declare free
    default 1
set free 2
declare after
    read-only
set after 'a'
set before 'b'
declare before
    read-only
log note '{version} {free} {after} {before}'
")]);
    // a read-only property may be assigned once, by a default or by a `set` either side of the declaration
    project.configure().ok().stderr("note: 3 2 a b");
}

#[test]
fn set_read_only_property() {
    let project = Project::new("read-only-set", &[("Build", "\
set host-target 'x'
declare version
    default 3
    read-only
set version 4
set early 'a'
set early 'b'
declare early
    read-only
set defaulted 'a'
declare defaulted
    default 'd'
    read-only
log note 'early is {early}'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: cannot set read-only builtin property \"host-target\"\n --> Build:1:5")
        .stderr("note: host-target: the target triple of the machine running the build\nerror")
        .stderr("error: cannot set read-only property \"version\"\n --> Build:5:5")
        .stderr("note: value assigned here\n --> Build:3:5")
        .stderr("note: declared read-only here\n --> Build:2:9")
        // only the first `set` before the declaration is permitted, and its value is kept
        .stderr("error: cannot set read-only property \"early\"\n --> Build:7:5")
        .stderr("note: value assigned here\n --> Build:6:5")
        .stderr("note: early is a")
        // a default leaves no room for an earlier `set`
        .stderr("error: cannot set read-only property \"defaulted\"\n  --> Build:10:5")
        .stderr("note: value assigned here\n  --> Build:12:5")
        .stderr("configuration failed with 4 errors");
}

#[test]
fn duplicate_declare() {
    let project = Project::new("declare-duplicate", &[("Build", "\
set later 1
declare later
declare size
    type 'unsigned'
    default 1
declare size
    type 'str'
    default 'x'
declare host-target
    default 'y'
log note 'size is {size}'
")]);
    project.configure().failed()
        .stderr("error: redeclaration of property \"size\"\n --> Build:6:9")
        .stderr("note: previously declared here\n --> Build:3:9")
        .stderr("error: redeclaration of builtin property \"host-target\"\n --> Build:9:9")
        .stderr("note: host-target: the target triple of the machine running the build\nnote: size is 1")
        // the attributes of a duplicate are ignored
        .stderr("note: size is 1")
        .stderr("configuration failed with 2 errors");
}

#[test]
fn meta_if() {
    let project = Project::new("meta-if", &[("Build", "\
declare debug
    default false
declare level
    default 2
@if debug
set level 9
@if !debug
log note 'release'
@if level > 1
@if {'true'}
crate kept 'x.rs'
    target host-target
@if false
crate skipped 'x.rs'
    target host-target
@if false
@if missing
log note 'not evaluated'
log note 'level is {level}'
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr("note: release").stderr("note: level is 2");
    assert!(!output.stderr.contains("not evaluated"), "{}", output.stderr);
    // the attributes of a skipped statement are skipped with it
    assert_eq!(output.outputs(), [format!("target/{host}/libkept.rlib")]);
}

#[test]
fn meta_if_errors() {
    let project = Project::new("meta-if-errors", &[("Build", "\
@if 1
log note 'unsigned condition'
@if true

log note 'after blank line'
@if true
# comment
log note 'after comment'
@elif true
log note 'unknown'
@if true
")]);
    let output = project.configure().failed();
    output
        .stderr("error: unsigned value used where bool is required\n --> Build:1:5")
        .stderr("error: expected statement on the line after a meta-statement\n --> Build:3:1")
        .stderr("error: expected statement on the line after a meta-statement\n --> Build:6:1")
        .stderr("error: unknown meta-statement \"@elif\"\n --> Build:9:1")
        .stderr("error: expected statement on the line after a meta-statement\n  --> Build:11:1")
        .stderr("configuration failed with 5 errors");
    // an invalid condition skips the statement
    assert!(!output.stderr.contains("note: unsigned condition"), "{}", output.stderr);
    assert!(!output.stderr.contains("note: unknown"), "{}", output.stderr);
}

#[test]
fn meta_else() {
    let project = Project::new("meta-else", &[("Build", "\
declare level
    default 2
@if level == 1
log note 'one'
@else
@if level == 2
log note 'two'
@else
log note 'other'

# blank lines and comments may separate the statements of a chain
@if level == 2
log note 'if taken'

@else
log note 'else not taken'
@if false
log note 'x'
@else
@if false
log note 'y'
@else
log note 'final else'
@if false
@if missing
log note 'skipped'
@else
crate fallback 'x.rs'
    target host-target
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr("note: two").stderr("note: if taken").stderr("note: final else");
    for skipped in ["one", "other", "else not taken", "note: x", "note: y", "skipped"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
    assert_eq!(output.outputs(), [format!("target/{host}/libfallback.rlib")]);
}

#[test]
fn meta_else_errors() {
    let project = Project::new("meta-else-errors", &[("Build", "\
@else
log note 'no if'
@if true
log note 'a'
log note 'unconditional'
@else
log note 'chain ended'
@if true
@else
log note 'not first'
@if 1
log note 'bad condition'
@else
log note 'else after bad condition'
@else x
log note 'trailing'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: \"@else\" must follow a conditional statement\n --> Build:1:1")
        // an unconditional statement ends the chain
        .stderr("error: \"@else\" must follow a conditional statement\n --> Build:6:1")
        .stderr("error: \"@else\" must be the first meta-statement of a statement\n --> Build:9:1")
        .stderr("error: unsigned value used where bool is required\n  --> Build:11:5")
        .stderr("error: expected \"\\n\"\n  --> Build:15:6")
        .stderr("configuration failed with 5 errors");
    // an invalid chain processes none of its statements
    for skipped in ["no if", "chain ended", "not first", "bad condition", "else after bad condition", "trailing"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
}

#[test]
fn meta_else_if() {
    let project = Project::new("meta-else-if", &[("Build", "\
declare level
    default 2
@if level == 1
log note 'one'
@else-if level == 2
log note 'two'
@else-if missing
log note 'not evaluated'
@else
log note 'other'
@if false
log note 'x'
@else-if {'false'}
log note 'y'
@else-if true
@if level > 1
log note 'nested condition'
")]);
    let output = project.configure().ok();
    output.stderr("note: two").stderr("note: nested condition");
    for skipped in ["one", "not evaluated", "other", "note: x", "note: y"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
}

#[test]
fn meta_else_if_errors() {
    let project = Project::new("meta-else-if-errors", &[("Build", "\
@else-if true
log note 'no if'
@if true
@else-if true
log note 'not first'
@if false
log note 'a'
@else-if 1
log note 'bad condition'
@else
log note 'else after bad condition'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: \"@else-if\" must follow a conditional statement\n --> Build:1:1")
        .stderr("error: \"@else-if\" must be the first meta-statement of a statement\n --> Build:4:1")
        .stderr("error: unsigned value used where bool is required\n --> Build:8:10")
        .stderr("configuration failed with 3 errors");
    for skipped in ["no if", "not first", "bad condition", "else after bad condition"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
}

#[test]
fn property_info() {
    let project = Project::new("property-info", &[("Build", "\
declare pages
    type 'unsigned'
    info 'the number of pages in the heap'
declare pages
set pages 'many'
declare later
    info 'declared without a value'
log note '{later}'
set host-target 'x'
declare bad
    info 5
declare plain
declare plain
")]);
    let output = project.configure().failed();
    output
        .stderr("error: redeclaration of property \"pages\"\n --> Build:4:9")
        .stderr("note: previously declared here\n --> Build:1:9\n  |\n1 | declare pages\n  |         ^^^^^\nnote: pages: the number of pages in the heap\n")
        .stderr("error: invalid data type for property \"pages\"")
        .stderr("error: no value for property \"later\"\n --> Build:8:12\n  |\n8 | log note '{later}'\n  |            ^^^^^\nnote: later: declared without a value\n")
        .stderr("note: host-target: the target triple of the machine running the build")
        .stderr("error: unsigned value used where str is required\n  --> Build:11:10")
        .stderr("error: redeclaration of property \"plain\"\n  --> Build:13:9")
        .stderr("configuration failed with 6 errors");
    // once for each diagnostic about "pages"
    assert_eq!(output.stderr.matches("note: pages:").count(), 2, "{}", output.stderr);
    // no note for a property without info
    assert!(!output.stderr.contains("note: plain:"), "{}", output.stderr);
}

#[test]
fn type_error_origin() {
    let project = Project::new("type-origin", &[("Build", "\
declare level
    type 'unsigned'
    default 2
declare name
    default 'x'
declare copy
    default name
declare count
    type 'unsigned'
set count copy
@if level
log note 'a'
@if level == name
log note 'b'
crate app 'x.rs'
    target level
@if {target} == 1
log note 'c'
")]);
    let output = project.configure().failed();
    let default_level = "note: unsigned value set here\n --> Build:3:13\n  |\n3 |     default 2\n  |             ^\n";
    // a reference to a reference is followed to where the data was written
    let default_name = "note: str value set here\n --> Build:5:13\n  |\n5 |     default 'x'\n  |             ^^^\n";
    output
        .stderr(&format!("error: invalid data type for property \"count\"\n  --> Build:10:11\n   |\n10 | set count copy\n   |           ^^^^\n{default_name}"))
        .stderr(&format!("error: unsigned value used where bool is required\n  --> Build:11:5\n   |\n11 | @if level\n   |     ^^^^^\n{default_level}"))
        .stderr(&format!("^^^^^^^^^^^^^\n{default_level}{default_name}"))
        .stderr(&format!("error: unsigned value used where str is required\n  --> Build:16:12\n   |\n16 |     target level\n   |            ^^^^^\n{default_level}"))
        .stderr("configuration failed with 5 errors");
    // a builtin value has no location
    let builtin = "str value cannot be coerced to unsigned\n  --> Build:17:5\n   |\n17 | @if {target} == 1\n   |     ^^^^^^^^\n";
    let after = &output.stderr[output.stderr.find(builtin).expect(&output.stderr) + builtin.len()..];
    assert!(!after.starts_with("note"), "{}", output.stderr);
}

#[test]
fn type_error_does_not_assign() {
    let project = Project::new("type-assign", &[("Build", "\
declare count
    type 'unsigned'
    default 3
set count 'many'
log note 'count is {count}'
set early 'a'
set early 5
declare early
    type 'unsigned'
log note 'early is {early}'
declare backwards
    default 'x'
    type 'unsigned'
declare offset
    type 'signed'
    default 4
@if offset > -1
log note 'offset is signed'
declare locked
    default 'bad'
    type 'bool'
    read-only
set locked true
log note 'locked is {locked}'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: invalid data type for property \"count\"\n --> Build:4:11")
        .stderr("note: declared here\n --> Build:1:9")
        .stderr("note: count is 3")
        // a value set before the declaration is checked by it
        .stderr("error: invalid data type for property \"early\"\n --> Build:6:11")
        .stderr("note: declared here\n --> Build:8:9")
        .stderr("note: early is 5")
        // a default is checked against a type declared after it
        .stderr("error: invalid data type for property \"backwards\"\n  --> Build:12:13")
        // an unsigned value is converted for a signed property
        .stderr("note: offset is signed")
        // an invalid default is not an assignment of a read-only property
        .stderr("error: invalid data type for property \"locked\"\n  --> Build:20:13")
        .stderr("note: locked is true")
        .stderr("configuration failed with 4 errors");
}

#[test]
fn default_is_visible_to_later_attributes() {
    let project = Project::new("default-visible", &[("Build", "\
declare pages
    default 4
    info 'the number of pages, {pages} by default'
declare pages
")]);
    project.configure().failed().stderr("note: pages: the number of pages, 4 by default");
}

#[test]
fn functions() {
    let project = Project::new("functions", &[("Build", "\
declare arch
    default 'RiscV'
declare board
declare fallback
    default 'virt'
log note '{upper(arch)} {lower(arch)} {lower({5})}'
log note 'board is {select(board, fallback)}'
set board 'qemu'
log note 'board is now {select(board, fallback)}'
@if starts-with(lower(arch), 'risc') && ends-with(arch, 'V') && !ends-with(arch, 'x')
log note 'nested calls'
declare path
set path env('PATH')
log note 'path is {path}'
")]);
    let output = project.configure().ok();
    output
        .stderr("note: RISCV riscv 5")
        // unset properties are skipped
        .stderr("note: board is virt")
        .stderr("note: board is now qemu")
        .stderr("note: nested calls")
        .stderr(&format!("note: path is {}\n", std::env::var("PATH").unwrap()));
}

#[test]
fn function_errors() {
    let project = Project::new("function-errors", &[("Build", "\
declare a
set a select(a, typo)
set a select(a)
set a select()
set a upper('a', 'b')
set a upper(5)
set a nope(1)
set a upper('a',' b')
set a env('BLUEMETAL_CONFIGURE_TEST_UNSET')
@if upper('a')
log note 'x'
")]);
    project.configure().failed()
        // a misspelt property is not ignored
        .stderr("error: property \"typo\" is undeclared\n --> Build:2:17")
        .stderr("error: no value for any of the properties \"a\"\n --> Build:3:7")
        .stderr("error: function \"select\" takes at least 1 argument, but 0 were given\n --> Build:4:7")
        .stderr("error: function \"upper\" takes 1 argument, but 2 were given\n --> Build:5:7")
        .stderr("error: unsigned value used where str is required\n --> Build:6:13")
        .stderr("error: unknown function \"nope\"\n --> Build:7:7")
        .stderr("error: expected \", \" or \")\"\n --> Build:8:16")
        .stderr("error: environment variable \"BLUEMETAL_CONFIGURE_TEST_UNSET\" is not set\n --> Build:9:11")
        .stderr("error: str value used where bool is required\n  --> Build:10:5")
        .stderr("configuration failed with 9 errors");
}

#[test]
fn strings_in_templates() {
    let project = Project::new("template-strings", &[("Build", "\
declare arch
    default 'riscv'
log note 'path is {env('PATH')}'
log note '{upper('x')} and {lower('{upper(arch)}')}'
log note 'nested {'a {'b {upper('c')}'}'}'
log note 'braces {'{{'}{'}}'} {{x}}'
@if starts-with('{arch}64', 'riscv6')
log note 'condition'
")]);
    let output = project.configure().ok();
    output
        .stderr(&format!("note: path is {}\n", std::env::var("PATH").unwrap()))
        .stderr("note: X and riscv\n")
        .stderr("note: nested a b C\n")
        .stderr("note: braces {} {x}\n")
        .stderr("note: condition\n");
}

#[test]
fn strings_in_templates_errors() {
    let project = Project::new("template-strings-errors", &[("Build", "\
log note 'bad {upper(5)} here'
log note 'inner {'{upper(5)}'}'
log note 'unclosed {'a'
")]);
    project.configure().failed()
        // spans within nested strings are relative to the file
        .stderr("error: unsigned value used where str is required\n --> Build:1:22")
        .stderr("error: unsigned value used where str is required\n --> Build:2:26")
        .stderr("error: invalid string template\n --> Build:3:20");
}

#[test]
fn run_function() {
    let project = Project::new("run", &[("Build", "\
declare name
    default 'world'
log note '{run('sh', '-c', 'echo \"hello $WHO\"; echo', WHO: name)}|'
log note '{run('printf', '%s-%s', 'a', {5})}'
@if starts-with(run('sh', '-c', 'pwd'), '/')
log note 'condition'
")]);
    let output = project.configure().ok();
    // trailing newlines are removed
    output.stderr("note: hello world|\n").stderr("note: a-5\n").stderr("note: condition\n");
}

#[test]
fn run_errors() {
    let project = Project::new("run-errors", &[("Build", "\
declare a
set a run('sh', '-c', 'echo oops >&2; echo second >&2; exit 3')
set a run('bluemetal-no-such-program')
set a run('echo', X: 'a', 'late')
set a run('echo', 5)
set a run(X: 'a')
set a run('sh', '-c', 'printf \"\\377\"')
")]);
    project.configure().failed()
        .stderr("error: \"sh\" failed with exit status: 3\n --> Build:2:7")
        // shown like a location
        .stderr("^\n --> stderr\n  |\n  | oops\n  | second\n  |\nerror")
        .stderr("error: failed to run \"bluemetal-no-such-program\": ")
        .stderr("error: program arguments must come before environment variables\n --> Build:4:27")
        .stderr("error: unsigned value used where str is required\n --> Build:5:19")
        .stderr("error: function \"run\" takes at least 1 argument, but 0 were given\n --> Build:6:7")
        .stderr("error: output of \"sh\" is not valid unicode\n --> Build:7:7")
        .stderr("configuration failed with 6 errors");
}

#[test]
fn path_function() {
    let project = Project::new("path", &[
        ("Build", "log note 'root {path('x.rs')} {path('./a/../../b')} {path('a/..')} {path('/x/../../y')}'\nuse 'sub'\n"),
        ("sub/Build", "\
declare abs
    default '/abs/file'
log note 'sub {path('x.rs')} {path('../y')} {path(abs)} {'{path('t')}'}'
log note '{path({5})}'
"),
    ]);
    project.configure().ok()
        // `..` is resolved, but kept if it leaves a relative path's start, and removed at the root of an absolute one
        .stderr("note: root x.rs ../b . /y\n")
        // relative to the file's directory, and an absolute path is unchanged
        .stderr("note: sub sub/x.rs y /abs/file sub/t\n")
        .stderr("note: sub/5\n");
}

#[test]
fn uppercase_identifiers() {
    let project = Project::new("uppercase", &[("Build", "\
declare Board_Name
    default 'virt'
set {'Arch_{Board_Name}'} 'riscv'
declare Arch_virt
log note '{Board_Name} {Arch_virt}'
crate Upper 'x.rs'
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr("note: virt riscv\n");
    output.build(&format!("target/{host}/libUpper.rlib"));
}

#[test]
fn run_environment_names() {
    let project = Project::new("run-env-names", &[("Build", "\
declare name
    default 'world'
log note '{run('sh', '-c', 'echo $NAME_world', {'NAME_{name}'}: 'computed')}'
log note '{run('sh', '-c', 'echo $CC $X-Y', CC: 'clang', name: name)}'
log note '{run('printenv', 'X-Y', X-Y: 'hyphen')}'
log note '{run('printf', {5})}|{run('printf', '%s', {'{5}'})}'
set name run('printenv', {'a b'}: 'x')
")]);
    project.configure().failed()
        // a name in `{}` is computed, like the name of a property
        .stderr("note: computed\n")
        .stderr("note: clang -Y\n")
        .stderr("note: hyphen\n")
        // an expression in `{}` is still an argument unless it is followed by `: `
        .stderr("note: 5|5\n")
        .stderr("error: invalid character ' ' in identifier \"a b\"\n --> Build:7:26")
        .stderr("configuration failed with 1 error");
}

#[test]
fn read_function() {
    let project = Project::new("read", &[
        ("Build", "use 'sub'\n"),
        ("sub/Build", "\
log note 'version {read('data dir/VERSION')}|'
log note '{read('plain')} {read('../sub/plain')}'
"),
        ("sub/data dir/VERSION", "1.2.3\n\n"),
        ("sub/plain", "abc"),
    ]);
    // trailing newlines are removed
    project.configure().ok().stderr("note: version 1.2.3|\n").stderr("note: abc abc\n");
    // the configuration is regenerated when a file it read changes, with special characters escaped
    let depfile = std::fs::read_to_string(project.dir.join("build.ninja.d")).unwrap();
    assert_eq!(depfile, "build.ninja: Build sub/Build sub/data\\ dir/VERSION sub/plain\n");
}

#[test]
fn read_errors() {
    let project = Project::new("read-errors", &[
        ("Build", "declare a\nset a read('missing')\nset a read('binary')\nset a read(5)\n"),
        ("binary", "\u{0}\u{0}"),
    ]);
    std::fs::write(project.dir.join("binary"), b"\xff").unwrap();
    project.configure().failed()
        .stderr("error: failed to read \"missing\": ")
        .stderr("\n --> Build:2:12")
        .stderr("error: failed to read \"binary\": stream did not contain valid UTF-8\n --> Build:3:12")
        .stderr("error: unsigned value used where str is required\n --> Build:4:12")
        .stderr("configuration failed with 3 errors");
}

#[test]
fn exists_function() {
    let project = Project::new("exists", &[
        ("Build", "use 'sub'\n"),
        ("sub/Build", "\
@if exists('present')
log note 'present'
@if !exists('absent')
log note 'absent'
@if exists('inner')
log note 'directory'
@if !exists('no/such/file')
log note 'deep'
@if exists('../sub/present')
log note 'normalized'
"),
        ("sub/present", ""),
        ("sub/inner/file", ""),
    ]);
    project.configure().ok()
        .stderr("note: present\n")
        .stderr("note: absent\n")
        .stderr("note: directory\n")
        .stderr("note: deep\n")
        .stderr("note: normalized\n");
    // only paths which exist are dependencies, as a missing dependency would always be out of date
    let depfile = std::fs::read_to_string(project.dir.join("build.ninja.d")).unwrap();
    assert_eq!(depfile, "build.ninja: Build sub/Build sub/present sub/inner\n");
}

#[test]
fn run_relative_program() {
    let project = Project::new("run-relative", &[
        ("Build", "use 'sub'\n"),
        ("sub/Build", "\
log note '{run('../tool.sh', 'arg')}'
log note '{run('./local.sh')}'
log note '{run('sh', '-c', 'echo from path')}'
"),
        ("tool.sh", "#!/bin/sh\necho \"tool $1\"\n"),
        ("sub/local.sh", "#!/bin/sh\necho local\n"),
    ]);
    use std::os::unix::fs::PermissionsExt;
    for script in ["tool.sh", "sub/local.sh"] {
        std::fs::set_permissions(project.dir.join(script), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // a program with a `/` is relative to the file, and the configuration depends on it
    project.configure().ok().stderr("note: tool arg\n").stderr("note: local\n").stderr("note: from path\n");
    // a program without a `/` is found in `PATH`, and is not a dependency
    let depfile = std::fs::read_to_string(project.dir.join("build.ninja.d")).unwrap();
    assert_eq!(depfile, "build.ninja: Build sub/Build tool.sh sub/local.sh\n");
}

#[test]
fn which_function() {
    let project = Project::new("which", &[("Build", "\
log note 'sh is {which('sh')}'
declare a
set a which('bluemetal-no-such-program')
set a which('a/b')
set a which('')
")]);
    let output = project.configure().failed();
    output
        .stderr("error: program \"bluemetal-no-such-program\" not found in PATH\n --> Build:3:13")
        .stderr("error: invalid program name \"a/b\", which cannot contain \"/\"\n --> Build:4:13")
        .stderr("error: invalid program name \"\"")
        .stderr("configuration failed with 3 errors");
    // the first executable `sh` in `PATH`
    let path = std::env::var_os("PATH").unwrap();
    let sh = std::env::split_paths(&path).map(|dir| dir.join("sh")).find(|path| path.is_file()).unwrap();
    output.stderr(&format!("note: sh is {}\n", sh.display()));
}

#[test]
fn try_function() {
    let project = Project::new("try", &[("Build", "\
declare a
    default 'x'
@if try(env('BLUEMETAL_CONFIGURE_TEST_UNSET'))
log note 'not taken'
@if !try(env('BLUEMETAL_CONFIGURE_TEST_UNSET') == 'a')
log note 'skipped the rest'
log note '{try(env('PATH'))} {try(upper(a))} {try(upper(5))} {try(missing)} {try(1 + )} {try(a, a)}'
log note 'nested {try(try(read('no file')))} paren {try(upper(')('))}'
")]);
    let output = project.configure().ok();
    output
        // errors are reported as warnings
        .stderr("warning: environment variable \"BLUEMETAL_CONFIGURE_TEST_UNSET\" is not set\n --> Build:3:13")
        // the rest of the argument is skipped after an error
        .stderr("note: skipped the rest\n")
        .stderr("warning: unsigned value used where str is required\n --> Build:7:57")
        .stderr("warning: no value for property \"missing\"")
        .stderr("warning: expected expression")
        .stderr("warning: expected \")\"")
        .stderr("note: true true false false false false\n")
        // a `try` succeeds even if its argument is a `try` which failed
        .stderr("note: nested true paren true\n")
        .stderr("configuration raised 7 warnings");
    assert!(!output.stderr.contains("not taken"), "{}", output.stderr);
}

#[test]
fn try_errors() {
    let project = Project::new("try-errors", &[("Build", "\
declare a
set a try()
set a try(upper('unclosed'
")]);
    // errors which leave the end of the argument unknown are not suppressed
    project.configure().failed()
        .stderr("error: function \"try\" takes 1 argument, but 0 were given\n --> Build:2:7")
        .stderr("error: expected \", \" or \")\"\n --> Build:3:27")
        .stderr("configuration failed with 2 errors");
}

#[test]
fn missing_paths_are_not_dependencies() {
    let project = Project::new("missing-dependencies", &[("Build", "\
@if try(run('./missing-program'))
log note 'ran'
@if try(read('missing-file'))
log note 'read'
")]);
    project.configure().ok().stderr("warning: failed to run \"./missing-program\"");
    let depfile = std::fs::read_to_string(project.dir.join("build.ninja.d")).unwrap();
    assert_eq!(depfile, "build.ninja: Build\n");
}

#[test]
fn log_expression() {
    let project = Project::new("log-expression", &[("Build", "\
declare level
    default 2
log note level * 3
log note upper('loud')
log warning level > 1
log note (level)
log note
")]);
    project.configure().failed()
        .stderr("note: 6\n --> Build:3:10\n  |\n3 | log note level * 3\n  |          ^^^^^^^^^\n")
        .stderr("note: LOUD\n")
        .stderr("warning: true\n")
        .stderr("note: 2\n")
        .stderr("error: expected \" \"\n --> Build:7:9")
        .stderr("configuration failed with 1 error, 1 warning");
}

#[test]
fn multi_line_message() {
    let project = Project::new("multi-line", &[("Build", "\
log warning run('printf', '%s\\n%s\\n\\n%s', 'first', 'second', 'after blank')
")]);
    // continuation lines are aligned with the first, except for blank lines
    project.configure().ok().stderr("\
warning: first
         second

         after blank
 --> Build:1:13
");
}

#[test]
fn rust_project() {
    let project = Project::new("rust-project", &[("Build", "\
crate core 'core.rs'
    builtin
crate util 'util.rs'
crate app 'app\"s.rs'
    use util
")]);
    let host = project.host();
    project.configure().ok();
    let json = std::fs::read_to_string(project.dir.join(".rust-project.json")).unwrap();
    let dir = std::path::absolute(&project.dir).unwrap();
    // crates are in the same order as their indices in `deps`, which match the `--extern` flags
    assert_contains(&json, &format!("\
        {{
            \"display_name\": \"app\",
            \"root_module\": \"{}/app\\\"s.rs\",
            \"edition\": \"2024\",
            \"target\": \"{host}\",
            \"is_workspace_member\": true,
            \"cfg\": [],
            \"build\": {{ \"label\": \"app@{host}\", \"build_file\": \"{}/Build\", \"target_kind\": \"lib\" }},
            \"deps\": [{{ \"crate\": 1, \"name\": \"core\" }}, {{ \"crate\": 2, \"name\": \"util\" }}]
        }},", dir.display(), dir.display()));
    assert_contains(&json, "\"display_name\": \"core\",");
    assert_contains(&json, &format!("\"root_module\": \"{}/util.rs\"", dir.display()));
    // the configure tool is checked as a test
    assert_contains(&json, &format!("\
            \"display_name\": \"configure\",
            \"root_module\": \"{}/dev/configure/main.rs\",
            \"edition\": \"2024\",
            \"target\": \"{host}\",
            \"is_workspace_member\": true,
            \"cfg\": [\"test\"],
            \"build\": {{ \"label\": \"configure\", \"build_file\": \"{}/bootstrap.ninja\", \"target_kind\": \"bin\" }},", dir.display(), dir.display()));
    assert_contains(&json, "\"sysroot_src\": \"");
    // the flycheck for a label replays its saved diagnostics
    assert_contains(&json, &format!("\
    \"runnables\": [
        {{
            \"kind\": \"flycheck\",
            \"program\": \"{}\",
            \"args\": [\"--check\", \"{{label}}\"],
            \"cwd\": \"{}\"
        }}
    ],", configure_path().display(), dir.display()));
}

#[test]
fn flycheck_replays_diagnostics() {
    let project = Project::new("flycheck", &[
        ("Build", "crate util 'util.rs'\ncrate app 'app.rs'\n    use util\n"),
        ("util.rs", "pub fn f() -> u32 { let unused = 1; 2 }\n"),
        ("app.rs", "pub fn g() -> u32 { util::f() }\n"),
    ]);
    project.bootstrap();
    let host = project.host();
    project.configure().ok();
    let ninja = project.configure().ninja;
    // checks are ordinary edges, which save their diagnostics
    assert_contains(&ninja, &format!("build check/app@{host}: phony target/{host}/check/libapp.rmeta\n"));
    assert_contains(&ninja, &format!("--extern util=target/{host}/check/libutil.rmeta"));

    // the diagnostics of the crates it depends on are replayed too
    let (first, _) = project.check(&format!("app@{host}"));
    assert_contains(&first, "\"message\":\"unused variable: `unused`\"");
    let util = project.dir.join(format!("target/{host}/check/util.json"));
    let modified = std::fs::metadata(&util).unwrap().modified().unwrap();

    // an up to date check is not run again, but its diagnostics are still replayed
    let (second, stderr) = project.check(&format!("app@{host}"));
    assert_eq!(second, first);
    assert!(!stderr.contains("CHECK"), "{stderr}");
    assert_eq!(std::fs::metadata(&util).unwrap().modified().unwrap(), modified);

    // a failed check is reported, and runs again next time
    std::fs::write(project.dir.join("app.rs"), "pub fn g() -> u32 { util::f() + \"x\" }\n").unwrap();
    let (failed, _) = project.check(&format!("app@{host}"));
    assert_contains(&failed, "\"message\":\"cannot add `&str` to `u32`\"");
    assert_contains(&failed, "\"message\":\"unused variable: `unused`\"");
    let (again, stderr) = project.check(&format!("app@{host}"));
    assert_eq!(again, failed);
    assert_contains(&stderr, "FAILED");
}

#[test]
fn flycheck_problems() {
    let project = Project::new("flycheck-problems", &[("Build", "crate app 'app.rs'\n"), ("app.rs", "")]);
    // without a configuration
    let (stdout, stderr) = project.check("app");
    assert_eq!(stdout, "");
    assert_contains(&stderr, "failed to read target/flycheck.db");
    // with an unknown label
    project.bootstrap();
    project.configure().ok();
    let (stdout, stderr) = project.check("nope");
    assert_eq!(stdout, "");
    assert_contains(&stderr, "no crate with the label \"nope\"");
}
