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

- `include <path>`: start processing the file `<path>`
- `crate <name>`: introduces a crate named `<name>`
