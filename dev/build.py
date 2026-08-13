from pathlib import Path
from typing import Literal
import dev
import dev.ninja
import importlib
import importlib.util
import pkgutil
import subprocess
import sys


class Target:
    def __init__(
        self,
        name,
        /,
        builtin: bool = False,
    ):
        self.name = name
        self.builtin = builtin


    def generate(self, writer: dev.ninja.Writer, root: Path, rule: 'Rule'):
        writer.build(rule.name, outputs=root / self.name)


class Rule:
    targets: set[Target]
    builtins: set[Target]
    def __init__(
        self,
        name: str,
        /,
    ):
        self.name = name
        self.targets = set()
        self.builtins = set()


    def generate(self, writer: dev.ninja.Writer):
        pass


    def add(self, target: Target):
        if target.name in self.targets:
            raise Exception(f"duplicate target `{target.name}`")
        self.targets.add(target)
        if target.builtin:
            self.builtins.add(target)
        return target


_sysroot = subprocess.run(['rustc', '--print=sysroot'], check=True, capture_output=True, encoding='utf-8').stdout.strip()
_rust_src = Path(_sysroot) / 'lib/rustlib/src/rust'
_host_tuple = subprocess.run(['rustc', '--print=host-tuple'], check=True, capture_output=True, encoding='utf-8').stdout.strip()

class Rust(Rule):
    def __init__(
        self,
        name: str,
        *,
        target: str | None = None,
        flags: list[str] | None = None,
    ):
        super().__init__(f"rust-{name}")
        self.info = name
        self.target = target or _host_tuple
        self.flags = flags or []


    def generate(self, writer: dev.ninja.Writer):
        writer.rule(
            self.name,
            'rustc',
            '-Dwarnings',
            '--color=always',
            '--edition=2024',
            f"--target={self.target}",
            '-Clinker-flavor=ld.lld',
            '--crate-name=$crate_name',
            '--crate-type=$crate_type',
            '--emit=dep-info=$depfile',
            '--emit=link=$out',
            *self.flags,
            '$flags',
            '$in',
            description="RUST $out",
            depfile='$out.d',
        )


class Crate(Target):
    def __init__(
        self,
        name: str,
        source: str,
        /,
        *,
        crate_type: Literal['bin', 'rlib'] = 'rlib',
        path: str | None = None,
        depends: list['Crate'] | None = None,
        builtin: bool = False,
        sysroot: bool = False,
    ):
        if '/' in name:
            raise ValueError('invalid crate name')
        self.crate_name = name
        if crate_type == 'rlib':
            name = f"lib{name}.rlib"
        if path is not None:
            name = f"{path}/{name}"
        super().__init__(name, builtin=builtin)
        self.source = source
        self.crate_type = crate_type
        self.depends = depends or []
        self.builtin = builtin
        self.sysroot = sysroot


    def generate(self, writer: dev.ninja.Writer, root: Path, rule: Rule):
        assert isinstance(rule, Rust)
        output = root / self.name
        flags = [
            f"--out-dir={output.parent}",
        ]
        deps = []
        if not self.builtin:
            for dep in rule.builtins:
                path = root / dep.name
                flags.append(f"--extern={dep.crate_name}={path}")
                deps.append(path)
        for dep in self.depends:
            path = root / dep.name
            flags.append(f"--extern={dep.crate_name}={path}")
            deps.append(path)
        source = self.source
        if self.sysroot:
            source = _rust_src / source
        writer.build(
            rule.name,
            outputs=output,
            inputs=source,
            dependencies=deps,
            crate_name=self.crate_name,
            crate_type=self.crate_type,
            flags=flags,
        )


class Linker(Rule):
    def __init__(
        self,
        name: str,
        *,
        program: str = 'ld.lld',
        flags: list[str] | None = None,
    ):
        super().__init__(name)
        self.program = program
        self.flags = flags or []


    def generate(self, writer: dev.ninja.Writer):
        writer.rule(
            self.name,
            'ld.lld',
            '-o',
            '$out',
            '--dependency-file=$depfile',
            *self.flags,
            '$flags',
            '$in',
            description='LINK $out',
            depfile='$out.d',
        )


class Link(Target):
    def __init__(
        self,
        name: str,
        /,
        *inputs: Target | str | Path,
    ):
        super().__init__(name)
        self.inputs = inputs


    def generate(self, writer: dev.ninja.Writer, root: Path, rule: Rule):
        assert isinstance(rule, Linker)
        output = root / self.name
        flags = []
        inputs = []
        for i in self.inputs:
            if isinstance(i, Target):
                inputs.append(root / i.name)
            else:
                inputs.append(i)
        writer.build(
            rule.name,
            outputs=output,
            inputs=inputs,
            flags=flags,
        )


class Project:
    source_dir: Path
    build_dir: Path

    build_files: list[Path]

    rules: dict[str, Rule]

    def __init__(
        self,
        source_dir: Path,
        build_dir: Path,
    ):
        self.source_dir = source_dir
        self.build_dir = build_dir
        self.build_files = []
        self.rules = {}


    def __enter__(self):
        return self


    def __exit__(self, exc_type, exc_value, traceback):
        if exc_value is None:
            self._generate()


    def _include(
        self,
        build_file: Path,
    ):
        name = 'buildgen'
        spec = importlib.util.spec_from_file_location(name, build_file)
        if spec is None:
            raise ImportError(name=name, path=build_file)
        buildgen = importlib.util.module_from_spec(spec)

        # Inject methods
        buildgen.crate = lambda name, path: self._crate(name, path)

        sys.modules[name] = buildgen
        spec.loader.exec_module(buildgen)
        self.build_files.append(build_file)


    def _generate(self):
        def module_deps(path: Path) -> list[str]:
            paths = []
            for info in [p for p in pkgutil.iter_modules(dev.__path__)]:
                p = (Path(info.module_finder.path) / info.name).with_suffix('.py')
                paths.append(str(p.relative_to(self.source_dir, walk_up=True)))
            return paths

        build_rel = self.build_dir.relative_to(self.source_dir, walk_up=True)

        deps = module_deps('Build')
        with open(self.source_dir / 'build.ninja', 'w') as f:
            writer = dev.ninja.Writer(f)
            writer.rule('dev', 'python', '$in', description='Regenerate $out')
            writer.build(
                'dev',
                outputs='build.ninja',
                inputs='Build',
                dependencies=[str(p) for p in self.build_files] + deps,
                pool='console',
            )
            for rule in sorted(self.rules.values(), key=lambda r: r.name):
                rule.generate(writer)

                for target in sorted(rule.targets, key=lambda t: t.name):
                    target.generate(writer, build_rel, rule)


    def add(self, rule: Rule, /):
        self.rules[rule.name] = rule
        return rule
