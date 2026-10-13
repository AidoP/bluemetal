## Building

Create the main `Build` file to configure the build system.

Typically, this will set some values then include a the build definition for a particular platform.

```
include arch/riscv/virt
```

Now the build system can be bootstrapped.

```
ninja -f bootstrap.ninja
```

From now on just build with `ninja`.


### Statements

- `use <path>`: start processing the build configuration file at `<path>`
- `declare <name>`: define a property that can be used for substitution
- `set <name> <value>`: assign a value to a property
- `crate <name>`: introduces a crate named `<name>`


### Builtin properties

- `sysroot`: the sysroot of the Rust toolchain, as printed by `rustc --print=sysroot`
