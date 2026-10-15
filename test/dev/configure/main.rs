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
        self.configure_with(&[])
    }
    /// Run `configure` with environment variables.
    fn configure_with(&self, env: &[(&str, &str)]) -> Output {
        let configure = configure_path();
        let output = Command::new(&configure)
            .arg("Build")
            .envs(env.iter().copied())
            .current_dir(&self.dir)
            .output()
            .unwrap_or_else(|error| panic!("failed to run {configure:?}: {error}"));
        Output {
            success: output.status.success(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            ninja: std::fs::read_to_string(self.dir.join("build.ninja")).unwrap_or_default(),
        }
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
    // the configure tool is only part of a project with its source
    assert!(!json.contains("\"display_name\": \"configure\""), "{json}");
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
    assert_contains(&stderr, "failed to read target/build.db");
    // with an unknown label
    project.configure().ok();
    let (stdout, stderr) = project.check("nope");
    assert_eq!(stdout, "");
    assert_contains(&stderr, "no crate with the label \"nope\"");
}

#[test]
fn custom_command() {
    let project = Project::new("custom", &[
        ("Build", "use 'sub'\n"),
        ("sub/Build", "\
custom copy
    input 'in.txt'
    output '../out/copy.txt'
    command 'sh' '-c' 'cat \"$1\" > \"$2\"; echo \"a  b\" >> \"$2\"' 'sh' input output
custom tool
    output '../out/tool.txt'
    command './tool.sh' output 'a b' 4 * 2
"),
        ("sub/in.txt", "hello\n"),
        ("sub/tool.sh", "#!/bin/sh\necho \"tool: $*\" > \"$1\"\n"),
    ]);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(project.dir.join("sub/tool.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = project.configure().ok();
    // `input` and `output` are the paths, and the arguments are quoted for `sh`, with `$` escaped for ninja
    assert_contains(output.build("out/copy.txt"), "\
build out/copy.txt: custom sub/in.txt
    command = 'sh' '-c' 'cat \"$$1\" > \"$$2\"; echo \"a  b\" >> \"$$2\"' 'sh' 'sub/in.txt' 'out/copy.txt'
    name = copy");
    // a program which is a path is relative to the file, and a dependency
    assert_contains(output.build("out/tool.txt"), "\
build out/tool.txt: custom | sub/tool.sh
    command = './sub/tool.sh' 'out/tool.txt' 'a b' '8'");
    // a command's outputs can be built by its name
    assert_contains(&output.ninja, "\nbuild custom/copy: phony out/copy.txt\n");
    let ninja = Command::new("ninja").args(["custom/copy", "custom/tool"]).current_dir(&project.dir).output().unwrap();
    assert!(ninja.status.success(), "{}", String::from_utf8_lossy(&ninja.stdout));
    assert_eq!(std::fs::read_to_string(project.dir.join("out/copy.txt")).unwrap(), "hello\na  b\n");
    assert_eq!(std::fs::read_to_string(project.dir.join("out/tool.txt")).unwrap(), "tool: out/tool.txt a b 8\n");
}

#[test]
fn custom_errors() {
    let project = Project::new("custom-errors", &[("Build", "\
custom a
    output 'x'
custom b
    command 'true'
custom c
    output 'y'
    command 'true'
    command 'false'
    bogus 1
custom c
    output 'z'
    command 'true'
custom d
    command 'echo' input
    input 'late'
    output 'w'
log note '{try(input)}'
")]);
    project.configure().failed()
        .stderr("error: missing required attribute \"command\"\n --> Build:1:8")
        .stderr("error: missing required attribute \"output\"\n --> Build:3:8")
        .stderr("error: attribute \"command\" cannot be repeated\n --> Build:8:5")
        .stderr("error: unknown custom attribute \"bogus\"\n --> Build:9:5")
        .stderr("error: redeclaration of custom command \"c\"\n  --> Build:10:8")
        .stderr("note: previously declared here\n --> Build:5:8")
        // `input` is only set by the attributes before it, and only within the statement
        .stderr("error: no value for property \"input\"\n  --> Build:14:20")
        .stderr("note: false")
        .stderr("configuration failed with 6 errors");
}

#[test]
fn default_targets() {
    let project = Project::new("default", &[("Build", "\
crate kept 'kept.rs'
    default
crate other 'other.rs'
custom made
    output 'made.txt'
    command 'touch' output
    default
custom skipped
    output 'skipped.txt'
    command 'touch' output
"), ("kept.rs", ""), ("other.rs", "")]);
    let host = project.host();
    let output = project.configure().ok();
    assert_contains(&output.ninja, &format!("\ndefault custom/made target/{host}/libkept.rlib\n"));
    // `ninja` without targets only builds the defaults, once it has recorded how `build.ninja` was generated
    Command::new("ninja").arg("build.ninja").current_dir(&project.dir).output().unwrap();
    let ninja = Command::new("ninja").arg("-n").current_dir(&project.dir).output().unwrap();
    let ninja = String::from_utf8(ninja.stdout).unwrap();
    assert_contains(&ninja, "CUSTOM made");
    assert_contains(&ninja, &format!("RUSTC target/{host}/libkept.rlib"));
    assert!(!ninja.contains("skipped") && !ninja.contains("other"), "{ninja}");

    // without defaults, there is no `default` statement, so everything is built
    let project = Project::new("no-default", &[("Build", "crate other 'other.rs'\n")]);
    assert!(!project.configure().ok().ninja.contains("\ndefault "));
}

#[test]
fn lists() {
    let project = Project::new("lists", &[("Build", "\
declare flags
    type 'list'
    default ['-O', 'dtb']
declare more
    default flags + ['-o', 1 + 1, true]
log note '{more} | {flags == ['-O', 'dtb']} | {[] == []} | {[[1], 'x']}'
log note run('printf', '%s,', flags, ['a b', []])
custom pack
    input ['a.dts', 'b.dtsi']
    input 'c.dtsi'
    output 'out.bin'
    command 'tool' flags '-o' output input
")]);
    let output = project.configure().ok();
    // as a string, the items of a list are separated by spaces
    output.stderr("note: -O dtb -o 2 true | true | true | 1 x\n");
    // each item of a list is a separate argument
    output.stderr("note: -O,dtb,a b,\n");
    assert_contains(output.build("out.bin"), "\
build out.bin: custom a.dts b.dtsi c.dtsi
    command = 'tool' '-O' 'dtb' '-o' 'out.bin' 'a.dts' 'b.dtsi' 'c.dtsi'");
}

#[test]
fn list_errors() {
    let project = Project::new("list-errors", &[("Build", "\
declare flags
    type 'list'
    default ['a']
declare bad
    type 'list'
    default 'x'
log note [1, 2
log note [1,2]
log note flags + 'x'
@if flags
log note 'x'
")]);
    project.configure().failed()
        .stderr("error: invalid data type for property \"bad\"\n --> Build:6:13")
        .stderr("error: expected \", \" or \"]\"\n --> Build:7:15")
        .stderr("error: expected \", \" or \"]\"\n --> Build:8:12")
        .stderr("error: mismatched operand types list and str\n --> Build:9:10")
        .stderr("note: list value set here\n --> Build:3:13")
        .stderr("error: list value used where bool is required\n  --> Build:10:5")
        .stderr("configuration failed with 5 errors");
}

#[test]
fn list_functions() {
    let project = Project::new("list-functions", &[("Build", "\
declare flags
    default ['-O', 'dtb', 2, -1]
log note '{len(flags)} {len([])} {len([[1, 2]])} {len('abc')} {len('')} {len('é')}'
log note '{contains(flags, 'dtb')} {contains(flags, 'x')} {contains(flags, 2)} {contains(flags, -1)} {contains([[1]], [1])}'
@if len(flags) > 3 && contains(flags, '-O')
log note 'condition'
declare a
set a len(5)
set a contains('abc', 'a')
set a contains(flags)
")]);
    project.configure().failed()
        // the length of a string is in bytes
        .stderr("note: 4 0 1 3 0 2\n")
        .stderr("note: true false true true true\n")
        .stderr("note: condition\n")
        .stderr("error: unsigned value used where list or str is required\n --> Build:8:11")
        .stderr("error: str value used where list is required\n --> Build:9:16")
        .stderr("error: function \"contains\" takes 2 arguments, but 1 was given")
        .stderr("configuration failed with 3 errors");
}

#[test]
fn tests() {
    let project = Project::new("tests", &[
        ("Build", "use 'sub'\n"),
        ("sub/Build", "\
custom gen
    output 'gen.txt'
    command 'sh' '-c' 'echo hello > \"$1\"' 'sh' output
test generated
    input 'gen.txt'
    command './check.sh' input
test fails
    command 'sh' '-c' 'echo some output; echo an error >&2; exit 3'
test passes
    command 'true'
"),
        ("sub/check.sh", "#!/bin/sh\ngrep -q hello \"$1\"\n"),
    ]);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(project.dir.join("sub/check.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = project.configure().ok();
    // a test's target builds its inputs and program, and `test` builds those of every test
    assert_contains(&output.ninja, "\nbuild test/generated: phony sub/gen.txt sub/check.sh\n");
    assert_contains(&output.ninja, "\nbuild test: phony test/generated test/fails test/passes\n");

    let run = |names: &[&str]| {
        let output = Command::new(configure_path()).arg("--test").args(names).current_dir(&project.dir).output().unwrap();
        (output.status.success(), String::from_utf8(output.stdout).unwrap(), String::from_utf8(output.stderr).unwrap())
    };
    // the inputs are built before the tests run, and the output of a failed test is shown
    let (success, stdout, stderr) = run(&[]);
    assert!(!success);
    assert_contains(&stdout, "\
test generated ... ok
test fails ... FAILED
test passes ... ok

---- fails ----
exit status: 3
some output
an error

test result: FAILED. 2 passed; 1 failed
");
    assert_contains(&stderr, "1 test failed");
    // tests can be chosen by name
    let (success, stdout, _) = run(&["passes", "generated"]);
    assert!(success);
    assert_contains(&stdout, "test passes ... ok\ntest generated ... ok\n\ntest result: ok. 2 passed; 0 failed\n");
    let (success, _, stderr) = run(&["nope"]);
    assert!(!success);
    assert_contains(&stderr, "no test named \"nope\"");
}

#[test]
fn test_errors() {
    let project = Project::new("test-errors", &[("Build", "\
test bad
    output 'x'
    default
    command 'true'
test a
    command 'true'
test a
    command 'false'
custom a
    output 'y'
    command 'true'
test empty
    input 'x'
")]);
    project.configure().failed()
        .stderr("error: unknown test attribute \"output\"\n --> Build:2:5")
        .stderr("error: unknown test attribute \"default\"\n --> Build:3:5")
        // tests and custom commands have separate names
        .stderr("error: redeclaration of test \"a\"\n --> Build:7:6")
        .stderr("error: missing required attribute \"command\"\n  --> Build:12:6")
        .stderr("configuration failed with 4 errors");
}

#[test]
fn crate_tests() {
    let project = Project::new("crate-tests", &[
        ("Build", "crate util 'util.rs'\ncrate app 'app.rs'\n    use util\n    test\n"),
        ("util.rs", "pub fn two() -> u32 { 2 }\n"),
        ("app.rs", "\
pub fn four() -> u32 { util::two() * 2 }
#[cfg(test)]
mod tests {
    #[test]
    fn works() { assert_eq!(super::four(), 4); }
    #[test]
    fn broken() { assert_eq!(super::four(), 5); }
}
"),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    // the test harness is built like the crate, with `--test`, and is registered as a test named after the crate
    assert_contains(output.build(&format!("target/{host}/test/app")), &format!(
        "--extern util=target/{host}/libutil.rlib --test",
    ));
    assert_contains(&output.ninja, &format!("\nbuild test/app: phony target/{host}/test/app\n"));
    let run = Command::new(configure_path()).arg("--test").current_dir(&project.dir).output().unwrap();
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(!run.status.success());
    assert_contains(&stdout, "test app ... FAILED\n");
    // the harness's own output is shown, where its tests run in any order
    assert_contains(&stdout, "test tests::works ... ok\n");
    assert_contains(&stdout, "test tests::broken ... FAILED\n");
    assert_contains(&stdout, "\ntest result: FAILED. 0 passed; 1 failed\n");
}

#[test]
fn crate_test_errors() {
    let project = Project::new("crate-test-errors", &[("Build", "\
crate k 'k.rs'
    target 'riscv64gc-unknown-none-elf'
    test
crate app 'app.rs'
    test
test app
    command 'true'
")]);
    let host = project.host();
    project.configure().failed()
        // other environments are not supported yet
        .stderr(&format!("error: tests for target \"riscv64gc-unknown-none-elf\" cannot be run, only those for the host target \"{host}\" or a target with `test`\n --> Build:2:12"))
        // the tests of crates are registered once the targets are known, after the statements
        .stderr("error: redeclaration of test \"app\"\n --> Build:4:7")
        .stderr("note: previously declared here\n --> Build:6:6");
}

#[test]
fn regenerates_itself() {
    // without the configure tool's source, `build.ninja` regenerates itself with the configure tool which made it
    let project = Project::new("regenerate", &[("Build", "crate app 'app.rs'\n"), ("app.rs", "")]);
    let output = project.configure().ok();
    assert_contains(&output.ninja, &format!("\
rule configure
    command = '{}' $in $out
", configure_path().display()));
    assert_contains(&output.ninja, "\nbuild build.ninja: configure Build\n");
    assert!(!output.ninja.contains("subninja"), "{}", output.ninja);
    // and ninja can use it on its own, regenerating it first as it has not recorded how it was made
    let host = project.host();
    let ninja = Command::new("ninja").arg(format!("target/{host}/libapp.rlib")).current_dir(&project.dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&ninja.stdout);
    assert!(ninja.status.success(), "{stdout}");
    assert_contains(&stdout, "CONFIGURE build.ninja");
    assert!(project.dir.join(format!("target/{host}/libapp.rlib")).exists());
}

#[test]
fn self_hosted() {
    // with the configure tool's source, `build.ninja` builds it, and its tests, as implicit crates
    let project = Project::new("self-hosted", &[
        ("Build", "crate core 'core.rs'\n    target host-target\n    builtin\n"),
        ("dev/configure/main.rs", "fn main() {}\n"),
        ("test/dev/configure/main.rs", ""),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    assert_contains(&output.ninja, "\
rule configure
    command = 'target/configure' $in $out
");
    assert_contains(&output.ninja, "\nbuild build.ninja: configure Build | target/configure\n");
    // the toolchain's standard library is used, rather than builtin crates
    assert_contains(output.build("target/configure"), &format!("\
build target/configure: rustc dev/configure/main.rs
    crate-name = configure
    crate-type = bin
    flags = --target={host} -L dependency=target/{host}"));
    assert_contains(output.build(&format!("target/{host}/test/configure")), "--test");
    // the integration tests need the configure tool, and have no output other than their harness
    assert_contains(&output.ninja, &format!(
        "\nbuild test/test_dev_configure: phony target/{host}/test/test_dev_configure target/configure\n",
    ));
    assert_contains(&output.ninja, &format!("\nbuild test/configure: phony target/{host}/test/configure\n"));
    assert!(!output.ninja.contains("libtest_dev_configure.rlib"), "{}", output.ninja);
    // a declared crate cannot take the place of one
    std::fs::write(project.dir.join("Build"), "crate configure 'x.rs'\n").unwrap();
    project.configure().failed()
        .stderr(&format!("error: crate \"configure\" for target \"{host}\" is reserved for the build system\n --> Build:1:7"));
}

/// Custom targets need a nightly toolchain, which the project uses but a temporary directory may not.
const NIGHTLY: &[(&str, &str)] = &[("RUSTUP_TOOLCHAIN", "nightly")];

#[test]
fn custom_target() {
    let project = Project::new("custom-target", &[("Build", "\
crate kernel 'kernel.rs'
    target 'my-kernel'
target my-kernel
    inherit 'riscv64gc-unknown-none-elf'
    code-model 'large'
    no-redzone
    target-feature ['+m', '+a']
    os 'bluemetal'
")]);
    let output = project.configure_with(NIGHTLY).ok();
    let spec = std::fs::read_to_string(project.dir.join("target/targets/my-kernel.json")).unwrap();
    // inherited fields are kept, replaced in place, or added at the end
    assert_contains(&spec, "{\n  \"arch\": \"riscv64\",\n");
    assert_contains(&spec, "\n  \"code-model\": \"large\",\n");
    assert_contains(&spec, "\n  \"features\": \"+m,+a\",\n");
    assert_contains(&spec, "\n  \"metadata\": {\n");
    assert!(spec.ends_with(",\n  \"disable-redzone\": true,\n  \"os\": \"bluemetal\"\n}\n"), "{spec}");
    // crates for the target use its specification
    assert_contains(output.build("target/my-kernel/libkernel.rlib"), "\
build target/my-kernel/libkernel.rlib: rustc kernel.rs | target/targets/my-kernel.json
    crate-name = kernel
    crate-type = lib
    flags = --target=target/targets/my-kernel.json -Zunstable-options -L dependency=target/my-kernel");

    // an unchanged specification is not written again, as every crate for the target depends on it
    let modified = std::fs::metadata(project.dir.join("target/targets/my-kernel.json")).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    project.configure_with(NIGHTLY).ok();
    let again = std::fs::metadata(project.dir.join("target/targets/my-kernel.json")).unwrap().modified().unwrap();
    assert_eq!(again, modified);
}

#[test]
fn custom_target_errors() {
    let project = Project::new("custom-target-errors", &[("Build", "\
target bad
    inherit 'riscv64gc-unknown-none'
    max-atomic-width 32
    code-model 5
    no-redzone 'yes'
    target-feature [1]
target riscv64gc-unknown-none-elf
    os 'x'
target bad
crate k 'k.rs'
    target 'nonexistent'
crate j 'j.rs'
    target 'later'
target later
    inherit 'x86_64-unknown-none'
")]);
    let output = project.configure_with(NIGHTLY).failed();
    output
        .stderr("error: unknown target \"riscv64gc-unknown-none\", did you mean \"riscv64gc-unknown-none-elf\"?\n --> Build:2:13")
        // only some fields can be replaced
        .stderr("error: unknown target attribute \"max-atomic-width\"\n --> Build:3:5")
        .stderr("error: unsigned value used where str is required\n --> Build:4:16")
        .stderr("error: str value used where bool is required\n --> Build:5:16")
        .stderr("error: list value used where str is required\n --> Build:6:20")
        .stderr("error: \"riscv64gc-unknown-none-elf\" is the name of a builtin target, which can be changed with `override`\n --> Build:7:8")
        .stderr("error: redeclaration of target \"bad\"\n --> Build:9:8")
        .stderr("error: unknown target \"nonexistent\"\n  --> Build:11:12")
        .stderr("configuration failed with 8 errors");
    // a target can be declared after the crates which use it
    assert!(!output.stderr.contains("\"later\""), "{}", output.stderr);
}

#[test]
fn multi_line_strings() {
    let project = Project::new("multi-line-strings", &[("Build", "\
declare base
    default '0x80000000'
target kernel
    inherit 'riscv64gc-unknown-none-elf'
    link-script '''
        ENTRY(_start)

        SECTIONS {{
            . = {base};
        }}
    '''
log note '''
    first
# a comment, which is skipped
      indented {upper('x')}
    # not a comment
'''
log note '''
    a
''' == 'a'
")]);
    let output = project.configure_with(NIGHTLY).ok();
    // the indentation of the lines is removed, except for any beyond it, and the lines are templates
    output.stderr("note: first\n        indented X\n      # not a comment\n").stderr("note: true\n");
    let spec = std::fs::read_to_string(project.dir.join("target/targets/kernel.json")).unwrap();
    assert_contains(&spec, "\"link-script\": \"ENTRY(_start)\\n\\nSECTIONS {\\n    . = 0x80000000;\\n}\"");
}

#[test]
fn multi_line_string_errors() {
    let project = Project::new("multi-line-string-errors", &[("Build", "\
log note '''
    ok
   bad
      fine
  '''
log warning 'after'
log note '''
    never ended
")]);
    project.configure().failed()
        .stderr("error: lines of a multi-line string must be indented by 4 spaces, or end it\n --> Build:3:1")
        .stderr("error: lines of a multi-line string must be indented by 4 spaces, or end it\n --> Build:5:1")
        .stderr("error: multi-line string is not terminated\n --> Build:1:10")
        // the statement after it is still read
        .stderr("warning: after\n --> Build:6:13")
        .stderr("error: multi-line string is not terminated\n --> Build:7:10")
        .stderr("configuration failed with 4 errors, 1 warning");
}

#[test]
fn crate_targets() {
    let project = Project::new("crate-targets", &[("Build", "\
crate core 'core.rs'
    target ['riscv64gc-unknown-none-elf', host-target]
    builtin
crate util 'util.rs'
    target ['riscv64gc-unknown-none-elf', host-target]
crate app 'app.rs'
    target ['riscv64gc-unknown-none-elf']
    use util
")]);
    let host = project.host();
    let output = project.configure().ok();
    // a crate is declared for each target, with its dependencies for the same target
    let mut outputs = output.outputs();
    outputs.sort();
    let mut expected = vec![
        "target/riscv64gc-unknown-none-elf/libapp.rlib".to_string(),
        "target/riscv64gc-unknown-none-elf/libcore.rlib".into(),
        "target/riscv64gc-unknown-none-elf/libutil.rlib".into(),
        format!("target/{host}/libcore.rlib"),
        format!("target/{host}/libutil.rlib"),
    ];
    expected.sort();
    assert_eq!(outputs, expected);
    assert_contains(output.build(&format!("target/{host}/libutil.rlib")), &format!(
        "--extern core=target/{host}/libcore.rlib",
    ));
}

#[test]
fn crate_target_errors() {
    let project = Project::new("crate-target-errors", &[("Build", "\
crate a 'a.rs'
    target ['riscv64gc-unknown-none-elf', 'riscv64gc-unknown-none-elf']
crate b 'b.rs'
    target ['x86_64-unknown-none', 5]
crate c 'c.rs'
    target [host-target, 'riscv64gc-unknown-none-elf']
    test
")]);
    let host = project.host();
    let output = project.configure().failed();
    output
        .stderr("error: target \"riscv64gc-unknown-none-elf\" is listed more than once\n --> Build:2:12")
        .stderr("error: list value used where str is required\n --> Build:4:12")
        // only the crate for the host target is tested
        .stderr(&format!("error: tests for target \"riscv64gc-unknown-none-elf\" cannot be run, only those for the host target \"{host}\" or a target with `test`\n --> Build:6:12"))
        .stderr("configuration failed with 3 errors");
}

#[test]
fn target_test_runners() {
    let project = Project::new("target-runners", &[("Build", "\
crate app 'app.rs'
    target ['vm', 'plain', host-target]
    test
target vm
    inherit 'riscv64gc-unknown-none-elf'
    test
    runner ['./run.sh', '-m', 'virt']
target plain
    inherit 'x86_64-unknown-none'
    test true
"), ("run.sh", "")]);
    let host = project.host();
    let output = project.configure_with(NIGHTLY).ok();
    // a test is named with its target unless it is the host target, and needs its runner
    assert_contains(&output.ninja, "\nbuild test/app@vm: phony target/vm/test/app run.sh\n");
    assert_contains(&output.ninja, "\nbuild test/app@plain: phony target/plain/test/app\n");
    assert_contains(&output.ninja, &format!("\nbuild test/app: phony target/{host}/test/app\n"));
    // the runner runs the harness, which is its last argument, while without one the harness runs directly
    let database = std::fs::read(project.dir.join("target/build.db")).unwrap();
    let strings = |strings: &[&str]| {
        let mut bytes = (strings.len() as u32).to_le_bytes().to_vec();
        for s in strings {
            bytes.extend((s.len() as u32).to_le_bytes());
            bytes.extend(s.as_bytes());
        }
        bytes
    };
    let contains = |needle: &[u8]| database.windows(needle.len()).any(|window| window == needle);
    assert!(contains(&strings(&["./run.sh", "-m", "virt", "target/vm/test/app"])));
    assert!(contains(&strings(&["./target/plain/test/app"])));
}

#[test]
fn target_test_runner_errors() {
    let project = Project::new("target-runner-errors", &[("Build", "\
crate lib 'lib.rs'
    target 'no-tests'
    test
target no-tests
    inherit 'x86_64-unknown-none'
    runner 'ignored'
target bad
    inherit 'x86_64-unknown-none'
    test 'yes'
    runner []
")]);
    let host = project.host();
    project.configure_with(NIGHTLY).failed()
        // a runner does not allow tests on its own
        .stderr(&format!("error: tests for target \"no-tests\" cannot be run, only those for the host target \"{host}\" or a target with `test`\n --> Build:2:12"))
        .stderr("error: str value used where bool is required\n --> Build:9:10")
        .stderr("error: list value used where str or a list of str is required\n  --> Build:10:12")
        .stderr("configuration failed with 3 errors");
}

#[test]
fn target_override() {
    let project = Project::new("target-override", &[("Build", "\
crate app 'app.rs'
    target ['riscv64gc-unknown-none-elf', host-target]
    test
target riscv64gc-unknown-none-elf
    override
    test
    runner ['qemu-system-riscv64', '-kernel']
")]);
    let host = project.host();
    let output = project.configure_with(NIGHTLY).ok();
    // a builtin target keeps its specification, but can be tested
    assert_contains(output.build("target/riscv64gc-unknown-none-elf/test/app"), "\
    flags = --target=riscv64gc-unknown-none-elf -L dependency=target/riscv64gc-unknown-none-elf --test");
    assert_contains(&output.ninja, "\nbuild test/app@riscv64gc-unknown-none-elf: phony target/riscv64gc-unknown-none-elf/test/app\n");
    assert_contains(&output.ninja, &format!("\nbuild test/app: phony target/{host}/test/app\n"));
    assert!(!project.dir.join("target/targets").exists());
}

#[test]
fn target_override_errors() {
    let project = Project::new("target-override-errors", &[("Build", "\
target riscv64gc-unknown-none-elf
    override
    inherit 'x86_64-unknown-none'
    os 'x'
    test
target riscv64gc-unknown-none
    override
target x86_64-unknown-none
    os 'x'
target riscv64gc-unknown-none-elf
    override
")]);
    project.configure_with(NIGHTLY).failed()
        // it cannot be used with the attributes which describe a specification
        .stderr("error: attribute \"inherit\" cannot be used with `override`\n --> Build:3:5")
        .stderr("error: attribute \"os\" cannot be used with `override`\n --> Build:4:5")
        .stderr("error: unknown target \"riscv64gc-unknown-none\", did you mean \"riscv64gc-unknown-none-elf\"?\n --> Build:6:8")
        .stderr("error: \"x86_64-unknown-none\" is the name of a builtin target, which can be changed with `override`\n --> Build:8:8")
        .stderr("error: redeclaration of target \"riscv64gc-unknown-none-elf\"\n  --> Build:10:8")
        .stderr("configuration failed with 5 errors");
}

#[test]
fn attribute_comments() {
    let project = Project::new("attribute-comments", &[("Build", "\
declare size
    # type 'str'
    type 'unsigned'
    #default 'x'
    default 1
# a comment
crate app 'app.rs'
    # use missing
    default
    #
log note 'size is {size}'
")]);
    let host = project.host();
    let output = project.configure().ok();
    // an indented comment comments out an attribute, without ending the statement
    output.stderr("note: size is 1\n");
    assert_contains(&output.ninja, &format!("\ndefault target/{host}/libapp.rlib\n"));

    // a comment must be at the indentation of an attribute
    let project = Project::new("attribute-comment-errors", &[("Build", "crate bad 'bad.rs'\n        # too deep\n")]);
    project.configure().failed()
        .stderr("error: expected identifier\n --> Build:2:5")
        .stderr("configuration failed with 1 error");
}

#[test]
fn meta_for() {
    let project = Project::new("meta-for", &[("Build", "\
declare targets
    default ['riscv64gc-unknown-none-elf', 'x86_64-unknown-none']
@for t in targets
crate k 'k.rs'
    target t
@for n in 1 2 [3, 4]
@if n % 2 == 0
log note 'even {n}'
@for a in 'x' 'y'
@for b in [1, 2]
log note '{a}{b}'
@for e in []
log note 'never'
@if false
@for f in [missing]
log note 'skipped'
log note 'after'
")]);
    let output = project.configure().ok();
    // the statement is processed for each value, with the meta-statements after the `@for`
    let mut outputs = output.outputs();
    outputs.sort();
    assert_eq!(outputs, ["target/riscv64gc-unknown-none-elf/libk.rlib", "target/x86_64-unknown-none/libk.rlib"]);
    output.stderr("note: even 2\n").stderr("note: even 4\n");
    assert!(!output.stderr.contains("even 1") && !output.stderr.contains("even 3"), "{}", output.stderr);
    // loops can be nested
    output.stderr("note: x1\n").stderr("note: x2\n").stderr("note: y1\n").stderr("note: y2\n");
    // without values, or in a skipped statement, nothing is processed or evaluated
    assert!(!output.stderr.contains("never") && !output.stderr.contains("skipped"), "{}", output.stderr);
    output.stderr("note: after\n");
}

#[test]
fn meta_for_errors() {
    let project = Project::new("meta-for-errors", &[("Build", "\
@for x 1
log note 'a'
@for x in
log note 'b'
@for x in 1

log note 'c'
log note '{x}'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: expected \"in\"\n --> Build:1:8")
        .stderr("error: expected \" \"\n --> Build:3:10")
        .stderr("error: expected statement on the line after a meta-statement\n --> Build:5:1")
        // the property is only set for the statement
        .stderr("error: no value for property \"x\"\n --> Build:8:12")
        .stderr("configuration failed with 4 errors");
    assert!(!output.stderr.contains("note: a\n") && !output.stderr.contains("note: b\n"), "{}", output.stderr);
}

#[test]
fn meta_then() {
    let project = Project::new("meta-then", &[("Build", "\
declare x
    default 0
@for a in 1 2
set x (x + a)
@then
log note 'inc {a}, total: {x}'
@if x > 5
log note 'big'
@then
log note 'also big'
@else
log note 'small'
@then
@if x == 3
log note 'three'
@then
log note 'still three'
@for a in []
log note 'never'
@then
log note 'never either'
log note 'end'
")]);
    let output = project.configure().ok();
    // a chained statement is repeated by `@for` with the first
    output.stderr("note: inc 1, total: 1\n").stderr("note: inc 2, total: 3\n");
    // and is skipped with it, while `@else` applies to the whole chain
    assert!(!output.stderr.contains("big"), "{}", output.stderr);
    output.stderr("note: small\n");
    // a chained statement can have its own meta-statements, which apply to the rest of the chain
    output.stderr("note: three\n").stderr("note: still three\n");
    assert!(!output.stderr.contains("never"), "{}", output.stderr);
    output.stderr("note: end\n");
}

#[test]
fn meta_then_errors() {
    let project = Project::new("meta-then-errors", &[("Build", "\
@then
log note 'a'
log note 'b'

@then
log note 'c'
@then

log note 'd'
@then x
log note 'e'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: \"@then\" must be directly after a statement\n --> Build:1:1")
        .stderr("note: b\n")
        .stderr("error: \"@then\" must be directly after a statement\n --> Build:5:1")
        .stderr("error: expected statement on the line after a meta-statement\n --> Build:7:1")
        .stderr("error: expected \"\\n\"\n  --> Build:10:6")
        .stderr("note: e\n")
        .stderr("configuration failed with 4 errors");
    assert!(!output.stderr.contains("note: a\n") && !output.stderr.contains("note: c\n"), "{}", output.stderr);
}

#[test]
fn crate_types() {
    let project = Project::new("crate-types", &[
        ("Build", "\
crate answer 'answer.rs'
    crate-type 'proc-macro'
crate app 'app.rs'
    crate-type 'bin'
    use answer
    use util
crate util 'util.rs'
    use answer
set target 'riscv64gc-unknown-none-elf'
crate kernel 'kernel.rs'
    use answer
"),
        ("answer.rs", "\
use proc_macro::TokenStream;
#[proc_macro]
pub fn answer(_: TokenStream) -> TokenStream { \"42\".parse().unwrap() }
"),
        ("app.rs", "fn main() { println!(\"{} {}\", answer::answer!(), util::twice()); }\n"),
        ("util.rs", "pub fn twice() -> u32 { answer::answer!() * 2 }\n"),
        ("kernel.rs", "#![no_std]\npub fn k() -> u32 { answer::answer!() }\n"),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    let answer = format!("target/{host}/{}answer{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    // a procedural macro is for the host target, even when another is the default, and is used by any target
    assert_contains(output.build(&answer), &format!("\
    crate-type = proc-macro
    flags = --target={host} -L dependency=target/{host} --extern proc_macro"));
    assert_contains(output.build("target/riscv64gc-unknown-none-elf/libkernel.rlib"), &format!(
        "--extern answer={answer}",
    ));
    assert_contains(output.build(&format!("target/{host}/app")), "crate-type = bin");
    // the binary is built and runs, using the procedural macro directly and through a library
    let ninja = Command::new("ninja").arg(format!("target/{host}/app")).current_dir(&project.dir).output().unwrap();
    assert!(ninja.status.success(), "{}", String::from_utf8_lossy(&ninja.stdout));
    let app = Command::new(project.dir.join(format!("target/{host}/app"))).output().unwrap();
    assert_eq!(String::from_utf8(app.stdout).unwrap(), "42 84\n");
    // checking uses the procedural macro itself, as rustc runs it
    let (stdout, _) = project.check(&format!("app@{host}"));
    assert_eq!(stdout, "");
    let json = std::fs::read_to_string(project.dir.join(".rust-project.json")).unwrap();
    assert_contains(&json, &format!(
        "\"is_proc_macro\": true,\n            \"proc_macro_dylib_path\": \"{}/{answer}\",",
        std::path::absolute(&project.dir).unwrap().display(),
    ));
}

#[test]
fn crate_type_errors() {
    let project = Project::new("crate-type-errors", &[("Build", "\
crate a 'a.rs'
    crate-type 'dylib'
crate b 'b.rs'
    crate-type 'proc-macro'
    target 'riscv64gc-unknown-none-elf'
crate tool 'tool.rs'
    crate-type 'bin'
crate c 'c.rs'
    use tool
crate d 'd.rs'
    crate-type 5
")]);
    let host = project.host();
    project.configure().failed()
        .stderr("error: invalid crate type \"dylib\", which must be \"lib\", \"bin\" or \"proc-macro\"\n --> Build:2:16")
        .stderr(&format!("error: procedural macros cannot be compiled for target \"riscv64gc-unknown-none-elf\", only the host target \"{host}\"\n --> Build:5:12"))
        .stderr("error: crate \"tool\" is a binary, which cannot be used by another crate\n --> Build:9:9")
        .stderr("error: unsigned value used where str is required\n  --> Build:11:16")
        .stderr("configuration failed with 4 errors");
}

#[test]
fn join_and_split() {
    let project = Project::new("join-split", &[("Build", "\
declare features
    default ['+m', '+a', 1, true]
log note join(features, ',')
log note join([], ',') == ''
log note join([['a', 'b'], 'c'], '/')
log note len(split('a,,b', ','))
log note split('a::b', '::') == ['a', 'b']
log note split('', ',') == ['']
log note join(split('x-y-z', '-'), '_')
declare a
set a split('a', '')
set a join('a', ',')
set a split(['a'], ',')
")]);
    project.configure().failed()
        // items are converted as in a string template
        .stderr("note: +m,+a,1,true\n")
        .stderr("note: true\n --> Build:4:10")
        .stderr("note: a b/c\n")
        // empty items are kept
        .stderr("note: 3\n")
        .stderr("note: true\n --> Build:7:10")
        .stderr("note: true\n --> Build:8:10")
        .stderr("note: x_y_z\n")
        .stderr("error: the separator cannot be empty\n  --> Build:11:18")
        .stderr("error: str value used where list is required\n  --> Build:12:12")
        .stderr("error: list value used where str is required\n  --> Build:13:13")
        .stderr("configuration failed with 3 errors");
}

#[test]
fn get_function() {
    let project = Project::new("get", &[("Build", "\
declare items
    default ['a', 'b', ['c']]
log note '{get(items, 0)} {get(items, 1)} {get(items, {'1'})}'
log note get(items, 2) == ['c']
log note '{get(items, 5, 'none')} {get(items, -1, 'negative')} {get([], 0, 7)}'
declare a
set a get(items, 3)
set a get(items, -1)
set a get(items)
set a get(items, 'x')
set a get('ab', 0)
set a get(items, 0, 1, 2)
")]);
    project.configure().failed()
        .stderr("note: a b b\n")
        .stderr("note: true\n")
        // the default is used for an index out of range
        .stderr("note: none negative 7\n")
        .stderr("error: index out of range for a list of length 3\n --> Build:7:18")
        .stderr("error: index out of range for a list of length 3\n --> Build:8:18")
        .stderr("error: function \"get\" takes 2 or 3 arguments, but 1 was given\n --> Build:9:7")
        .stderr("error: str value used where integer is required\n  --> Build:10:18")
        .stderr("error: str value used where list is required\n  --> Build:11:11")
        .stderr("error: function \"get\" takes 2 or 3 arguments, but 4 were given\n  --> Build:12:7")
        .stderr("configuration failed with 6 errors");
}

#[test]
fn bitwise_operators() {
    let project = Project::new("bitwise", &[("Build", "\
declare flags
    default 0x0f
log note '{flags & 0x3} {flags | 0x30} {flags ^ 0xff} {1 << 4} {0x100 >> 4} {!0} {!flags & 0xf0}'
log note '{1 + 2 << 1} {1 | 2 & 3} {6 & 3 == 2} {-8 >> 1} {!-1} {-1 & 1}'
log note '{1 << 63 >> 63} {0x8000000000000000 << 1} {!{0}} {!true}'
log note '{1 << 64}'
log note '{1 << -1}'
log note '{true & 1}'
")]);
    project.configure().failed()
        // `!` is the complement of an integer
        .stderr("note: 3 63 240 16 16 18446744073709551615 240\n")
        // the precedence is the same as in Rust, and `>>` keeps the sign
        .stderr("note: 6 3 true -4 0 1\n")
        // bits shifted out are lost, while `{}` is taken as a boolean by `!`
        .stderr("note: 1 0 true false\n")
        .stderr("error: shift amount must be from 0 to 63\n --> Build:6:17")
        .stderr("error: shift amount must be from 0 to 63\n --> Build:7:17")
        .stderr("error: bool value used where integer is required\n --> Build:8:12")
        .stderr("configuration failed with 3 errors");
}

#[test]
fn min_max() {
    let project = Project::new("min-max", &[("Build", "\
declare sizes
    default [4, 16, 2]
declare offset
    type 'signed'
    default -5
declare unsigned
    type 'unsigned'
    default 1
log note '{min(sizes)} {max(sizes)} {min(3)} {max(1, 7, 3)} {min(sizes, 1)}'
log note '{max([3, -1])} {min(offset, 2)} {max(offset, [0])} {max([1], [9, 2], 3)} {min({'4'}, 6)}'
log note '{min()}'
log note '{min([])}'
log note '{min('a')}'
log note '{min(offset, unsigned)}'
")]);
    project.configure().failed()
        // each item of a list is a value
        .stderr("note: 2 16 3 7 1\n")
        // literals, including in lists, can be signed or unsigned
        .stderr("note: 3 -5 0 9 4\n")
        .stderr("error: function \"min\" takes at least 1 argument, but 0 were given\n  --> Build:11:12")
        .stderr("error: there are no values to compare\n  --> Build:12:12")
        .stderr("error: str value used where integer is required\n  --> Build:13:16")
        .stderr("error: mismatched operand types signed and unsigned\n  --> Build:14:12")
        .stderr("configuration failed with 4 errors");
}
