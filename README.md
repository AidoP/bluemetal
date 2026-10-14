## Building

Create the main `Build` file to configure the build system.

Typically, this will set some values then include a the build definition for a particular platform.

```
include arch/riscv/virt
```

Now the build system can be bootstrapped, which builds the configure tool, `target/configure`, and runs it to generate
`build.ninja`. This is only needed once.

```
ninja -f bootstrap.ninja
```

From now on just build with `ninja`, which rebuilds the configure tool and regenerates `build.ninja` when needed. Without targets, it builds the crates and custom commands with a `default`
attribute, or everything if there are none.

Configuring also writes `.rust-project.json`, which describes the crates to rust-analyzer. Its flycheck runs
`target/configure --check <label>`, which brings the crate's checks up to date with ninja, then prints their saved
diagnostics, so an up to date crate keeps its warnings without being checked again. The configuration is not evaluated
again for a flycheck, which reads what it needs from the build database, `target/build.db`.

Run the tests with `target/configure --test`, or only some of them with `target/configure --test <name> ...`. What
they need is built with ninja first, then each test runs from the project root, and its output is shown if it fails.
The configure tool's own tests are the `configure` and `test_dev_configure` tests, as the build system implicitly
declares its crates, which are not passed builtin crates.


### Statements

- `use <path>`: start processing the build configuration file at `<path>`, unless it has already been processed. A file which uses itself, directly or through other files, raises a warning
- `declare <name>`: define a property that can be used for substitution
- `set <name> <value>`: assign a value to a property
- `crate <name> [<path>]`: introduce a Rust library crate named `<name>`, with its root at `<path>`, or at
  `<name>/lib.rs` relative to the file without one
- `log <severity> <expression>`: report the value of an expression as an `error`, `warning` or `note`
- `custom <name>`: a custom build command, which ninja runs when its inputs change. Its outputs can be built with
  `ninja custom/<name>`
- `target <name>`: a custom target, which crates can be compiled for. See below
- `test <name>`: a test, which passes if its command succeeds. Like `custom`, it has `input` and `command` attributes,
  but no `output` or `default`. Its inputs are built with `ninja test/<name>`, or those of every test with `ninja test`

A `<name>` is an identifier: ASCII letters, digits, `-` and `_`, not starting with a digit. Crate names cannot
contain `-`, as rustc does not accept it. A name can instead be computed with an expression in `{}`, whose value must
be a valid name.

```
crate compiler_builtins
set {'heap-{arch}'} 0x1000
```


A line starting with `#` is a comment. Indented by 4 spaces, a comment can be among the attributes of a statement, to
comment out one of them.

```
crate app
    # use util
    default
```


### Meta-statements

A meta-statement is a line starting with `@` which applies to the statement on the next line. There cannot be a blank
line or a comment between them, but several meta-statements can apply to the same statement.

- `@if <expression>`: skip the statement, including its attributes, unless the expression is `true`. The expression
  must be a `bool`, or coerced to one with `{}`. Once a statement is skipped, the conditions of later meta-statements
  are not evaluated

```
@if {device-tree}
@if target == 'riscv64gc-unknown-none-elf'
use 'dtb'
```

- `@else`: process the statement only if no statement has been processed in the chain of conditional statements before
  it. A chain starts with a statement with `@if` and is continued by each statement with `@else`, so `@else` followed
  by `@if` is an else-if. Blank lines and comments may separate the statements of a chain, but another statement ends
  it. `@else` must be the first meta-statement of its statement
- `@else-if <expression>`: the same as `@else` followed by `@if <expression>`

```
@if level == 1
log note 'one'
@else-if level == 2
log note 'two'
@else
log note 'many'
```

- `@for <name> in <value> ...`: process the meta-statements and statement after it once for each of the values, which
  are expressions separated by spaces, where each item of a list is a value. For each, the property `<name>` has the
  value, within the file and until the end of the statement. Without any values, the statement is skipped

```
@for target in targets
@if target != host-target
crate kernel 'kernel.rs'
    target target
```

- `@then`: chain the statement after it to the statement before it, which it must be directly after, so that the
  statements are processed or skipped together, and repeated together by `@for`. The chained statement can have its
  own meta-statements, which apply to it and the rest of the chain

```
@for a in 1 2
set total (total + a)
@then
log note 'added {a}, the total is {total}'
```


### Expressions

An expression is a string, `true`, `false`, an integer, a list or the name of a property.

A list is written as expressions in `[]`, separated by `, `, such as `['-O', 'dtb']`, and its items can have any type.
Lists can be joined with `+` and compared with `==` and `!=`. As a string, such as in a string template, the items of
a list are separated by spaces, while each item is a separate argument of a command, such as for `run`.

Integers and integer properties can be combined with `+`, `-`, `*`, `/` and `%`, with a single space on either side
of the operator, and grouped with `(` and `)`. Both operands must have the same type, `unsigned` or `signed`.

Integers can be compared with `<`, `<=`, `>` and `>=`, and any two values of the same type with `==` and `!=`.
A `bool` can be negated with a leading `!` and combined with `&&` and `||`. From lowest to highest, the precedence is
`||`, `&&`, comparisons, `+` and `-`, then `*`, `/` and `%`.

An expression written in `{}` is coerced to the type the operator requires, or to the type of the other operand.
This converts between `unsigned` and `signed`, parses a `str` as an integer or as `true` or `false`, treats a `bool`
as `0` or `1`, and treats an integer as `true` unless it is zero. Integer literals are only coerced between
`unsigned` and `signed`.

```
set size {pages} * 0x1000 - 1
set large size >= 0x10_0000 && !{small}
```

Functions are called with their arguments in `()`, separated by `, `:

- `upper(str)` and `lower(str)`: convert a `str` to upper or lower case
- `starts-with(str, prefix)` and `ends-with(str, suffix)`: whether a `str` starts or ends with another, as a `bool`
- `select(name, ...)`: the value of the first of the named properties which has a value. Each property must be
  declared, or set before it is declared, so that a misspelt name is an error rather than ignored
- `env(name)`: the value of an environment variable, which must be set. The build is not reconfigured when it changes
- `len(value)`: the number of items in a list, or of bytes in a `str`, which is UTF-8, as an `unsigned`
- `contains(list, value)`: whether a list has an item equal to a value, as a `bool`
- `path(str)`: a path relative to the directory of the file containing the expression, as a `str`. An absolute path
  is not made relative. `.` and `..` are resolved without following symbolic links
- `read(path)`: the contents of a file, without trailing newlines, where the path is relative to the file containing
  the expression. The build is reconfigured when the file changes
- `exists(path)`: whether a file or directory exists, as a `bool`, where the path is relative to the file containing
  the expression. The build is reconfigured when an existing path changes, but not when a missing one is created
- `try(expression)`: whether the expression is evaluated without errors, as a `bool`. Its errors are reported as
  warnings instead, and its value is discarded

```
@if try(env('CROSS_COMPILE'))
set prefix env('CROSS_COMPILE')
```

- `which(name)`: the path of the first executable file with the given name in the directories of the `PATH` environment
  variable. The build is reconfigured when one of the directories searched before it changes
- `run(program, argument, ..., NAME: value, ...)`: run a program and return its output, without trailing newlines.
  As in `sh`, a program containing a `/` is a path, relative to the file containing the expression, while any other
  program is found in `PATH`. It inherits the environment, with any variables given as `NAME: value` after the
  arguments, where `NAME` is a name like those of properties, so it can be computed with `{}`. The configuration fails
  if the program exits unsuccessfully. The build is reconfigured when a program given as a path changes, but not when
  its output would change otherwise

```
set board select(board, default-board)
@if starts-with(target, 'riscv')
use 'arch/riscv'
```

Within a string, an expression in `{}` is replaced with its value, and `{{` and `}}` with a single brace.
The expression cannot start with `{`, but can contain strings, which are templates themselves.

```
log note 'size is {size} with {size / 0x1000} pages'
log note 'home is {env('HOME')}'
```

A multi-line string starts with `'''` at the end of a line and ends with `'''` at the indentation of that line. The lines
between are indented by 4 spaces more, which is removed, or are blank, and are joined by newlines, without one at the
end. A line starting with `#`, in the first column, is a comment which is skipped. Like other strings, each line is a
template.

```
log note '''
    first line
      indented line
'''
```

A `declare` statement may be followed by indented attributes:

- `type <type>`: the type of the property, one of `'str'`, `'bool'`, `'unsigned'`, `'signed'` or `'list'`
- `default <value>`: the initial value of the property
- `info <string>`: a description of the property, included in diagnostics about it
- `read-only`: the property can only be assigned once, by its default or by a single `set` before or after the declaration

A `crate` statement may be followed by indented attributes:

- `target <string>`: the target to compile the crate for, which is a builtin target or one declared by a `target`
  statement, or a list of them to declare the crate for each. Without it, the `target` property at the point of the
  declaration is used. A crate can be declared once for each target
- `use <name>`: a crate for the same target which the crate depends on, passed to rustc with `--extern`. It can be
  repeated, and the crate can be declared later or in another file
- `builtin`: pass the crate with `--extern` to every crate for the same target which is not builtin, like the standard
  library. Builtin crates are not passed to each other, so a builtin crate must `use` the ones it depends on
- `default`: build the crate when `ninja` is run without targets
- `test`: build the crate's tests with `--test` as `target/<target>/test/<name>`, and register a test which runs them,
  named after the crate, or as `<name>@<target>` if the target is not the host target. Only crates for the host target,
  or a custom target with `test`, can be tested

```
crate core '{rust-src}/library/core/src/lib.rs'
    builtin

crate compiler_builtins
    builtin
    use core

crate build_tool
    target host-target
```

A `custom` statement is followed by indented attributes:

- `input <path>`: a file the command reads, or a list of them, relative to the file containing the statement. It can be
  repeated
- `output <path>`: a file the command writes, or a list of them, relative to the file containing the statement. It is
  required, and can be repeated
- `command <program> <argument> ...`: the command, as expressions separated by spaces, each of which is a single
  argument, except for a list, each item of which is an argument. As with `run`, a program containing a `/` is a path relative to the file, and the outputs are rebuilt when
  it changes, while any other program is found in `PATH`. It is required
- `default`: run the command when `ninja` is run without targets

While the attributes are evaluated, the `input` and `output` properties are lists of the paths of the inputs and
outputs before them.

```
custom device-tree
    input 'virt.dts'
    output 'virt.dtb'
    command 'dtc' '-O' 'dtb' '-o' output input
```

A `target` statement is followed by indented attributes, which describe the target with a JSON target specification,
`target/targets/<name>.json`. Custom targets need a nightly toolchain.

- `inherit <target>`: the builtin target whose specification it is based on
- `override`: set the attributes of the builtin target with the name, rather than declaring a target. Only `test` and
  `runner` can be used with it, as the builtin target keeps its specification
- `env`, `os`, `arch`, `linker <str>`: replace the field of the specification
- `code-model`, `relocation-model`, `target-cpu`, `linker-flavor`, `panic <str>`: replace the field of the
  specification for the codegen option
- `target-feature <features>`: the features of the target, as a `str` separated by `,`, or a list of them
- `no-redzone`: disable the red zone
- `link-script <str>`: the linker script, which is usually a multi-line string, where braces must be doubled
- `test <bool>`: whether crates for the target can be tested, which is `false` by default. Without a value, it is `true`
- `runner <program>`: the program which runs a test harness for the target, given as its last argument, or a list of the
  program and its first arguments. As with `run`, a program containing a `/` is a path relative to the file. Without
  it, a test harness is run directly

```
target riscv64gc-unknown-none-elf
    override
    test
    runner ['qemu-system-riscv64', '-machine', 'virt', '-nographic', '-kernel']

target bluemetal-riscv64
    inherit 'riscv64gc-unknown-none-elf'
    os 'bluemetal'
    code-model 'medium'
    link-script '''
        ENTRY(_start)
        SECTIONS {{
            . = 0x80000000;
        }}
    '''
```

### Builtin properties

- `sysroot`: the sysroot of the Rust toolchain, as printed by `rustc --print=sysroot`
- `target`: the target triple for crates without a `target` attribute, initially the host
- `host-target`: the target triple of the host, as printed by `rustc --print=host-tuple`, which is read-only
