from pathlib import Path
from typing import Literal
import dev
from dev.config import Config
import dev.ninja
import dev.rust_analyzer
import pkgutil
import subprocess


class Crate:
    def __init__(
        self,
        name: str,
        module: Path,
        output: Path,
        crate_type: Literal['bin', 'lib'] = 'lib',
        /,
        *,
        target: str,
        deps: list['Crate'] = [],
        builtin: bool = False,
    ):
        self.name = name
        self.module = module
        self.output = output
        self.crate_type = crate_type
        self.target = target
        self.deps = deps
        self.builtin = builtin


    def _ninja(self, writer: dev.ninja.Writer, flycheck_writer: dev.ninja.Writer, project: 'Project'):
        flags = []
        deps = []
        if not self.builtin:
            for crate in project.builtins:
                path = project.build / crate.output
                flags.append(f"--extern={crate.name}={path}")
                deps.append(path)
        for crate in self.deps:
            path = project.build / crate.output
            flags.append(f"--extern={crate.name}={path}")
            deps.append(path)
        writer.build(
            'crate',
            outputs=project.build / self.output,
            inputs=self.module,
            dependencies=deps,
            crate_type=self.crate_type,
            crate_name=self.name,
            target=self.target,
            flags=flags,
        )
        flycheck_writer.build(
            'crate',
            outputs='target' / self.output,
            inputs=self.module,
            dependencies=deps,
            crate_type=self.crate_type,
            crate_name=self.name,
            target=self.target,
            flags=flags,
        )


    def _rust_analyzer(self, rust_analyzer: dev.rust_analyzer.Project, project: 'Project'):
        deps = set()
        if not self.builtin:
            for crate in project.builtins:
                deps.add(crate.name)
        for crate in self.deps:
            deps.add(crate.name)
        rust_analyzer.crate(
            self.module,
            self.name,
            edition='2024',
            deps=deps,
            build_info=dev.rust_analyzer.BuildInfo(
                'target' / self.output,
                self.module,
                self.crate_type,
            ),
        )


class Project:
    includes: list[Path]
    crates: dict[str, Crate]
    builtins: set[Crate]

    def __init__(
        self,
        root: Path,
        build: Path,
        /,
        config: Path | None = None,
    ):
        self.root = root
        self.build = build
        self.config_path = config
        self.includes = []
        self.crates = {}
        self.builtins = set()
        self.config = Config()


    def __enter__(self):
        self.config.load(self.config_path)
        self.sysroot = subprocess.run(['rustc', '--print=sysroot'], check=True, capture_output=True, encoding='utf-8').stdout.strip()
        self.rust_src = Path(self.sysroot) / 'lib/rustlib/src/rust'
        self.host_target = subprocess.run(['rustc', '--print=host-tuple'], check=True, capture_output=True, encoding='utf-8').stdout.strip()
        self.target = self.config.platform.target

        return self


    def __exit__(self, exc_type, exc_value, exc_tbk):
        if exc_type is not None:
            return

        def module_deps(path: Path) -> list[str]:
            paths = []
            for info in [p for p in pkgutil.iter_modules(dev.__path__)]:
                p = (Path(info.module_finder.path) / info.name).with_suffix('.py')
                paths.append(str(p.relative_to(self.root, walk_up=True)))
            return paths

        deps = module_deps('Build')

        build_ninja_path = self.root / 'build.ninja.tmp'
        check_ninja_path = self.root / 'check.ninja.tmp'
        with open(build_ninja_path, 'w') as build_ninja,  open(check_ninja_path, 'w') as check_ninja:
            build_writer = dev.ninja.Writer(build_ninja)
            check_writer = dev.ninja.Writer(check_ninja)
            check_writer.subninja(build_ninja_path.with_suffix(''))
            config_path = self.config_path.absolute().relative_to(self.root, walk_up=True)
            build_path = self.build.absolute().relative_to(self.root, walk_up=True)
            build_writer.rule(
                'configure',
                'python',
                'Configure',
                '-c',
                '$in',
                build_path,
                description='Regenerate $out',
            )
            build_writer.build(
                'configure',
                outputs=['build.ninja', 'check.ninja'],
                inputs=config_path,
                dependencies=['Configure'] + [str(p) for p in self.includes] + deps + self.config.includes,
                pool='console',
            )
            build_writer.rule(
                'crate',
                'rustc',
                # '-Dwarnings',
                '--color=always',
                '--edition=2024',
                '--target=$target',
                '--crate-name=$crate_name',
                '--crate-type=$crate_type',
                '--emit=dep-info=$depfile',
                '--emit=link=$out',
                '$flags',
                '$in',
                description="CRATE $crate_name",
                depfile='$out.d',
            )
            check_writer.rule(
                'crate',
                'rustc',
                '--edition=2024',
                '--target=$target',
                '--crate-name=$crate_name',
                '--crate-type=$crate_type',
                '--emit=metadata=$out.rmeta',
                '--error-format=json',
                '$flags',
                '$in',
                '||',
                'true',
                description="FLYCHECK $crate_name",
                pool='console',
            )

            for crate in sorted(self.crates.values(), key=lambda crate: (crate.target, crate.module)):
                crate._ninja(build_writer, check_writer, self)

        rust_project_path = self.root / '.rust-project.json.tmp'
        project = dev.rust_analyzer.Project()
        project.runnable(
            'ninja',
            '--quiet',
            '-f',
            'check.ninja',
            '{label}',
            cwd=str(self.root),
            kind='flycheck',
        )
        for crate in sorted(self.crates.values(), key=lambda crate: (crate.target, crate.module)):
            crate._rust_analyzer(project, self)
        project.write(rust_project_path)

        # commit files
        build_ninja_path.rename(build_ninja_path.with_suffix(''))
        check_ninja_path.rename(check_ninja_path.with_suffix(''))
        rust_project_path.rename(rust_project_path.with_suffix(''))


    def crate(
        self,
        name: str,
        crate_type: Literal['bin', 'lib'] = 'lib',
        /,
        *,
        path: str | None = None,
        edition: Literal['2021', '2024'] = '2024',
        target: str | Literal['native'] | None = None,
        builtin: bool = False,
        sysroot: bool = False,
        deps: list[Crate] = [],
    ) -> Crate:
        output = Path(name)
        module = path and Path(path)
        name = output.stem

        match target:
            case 'native':
                target = self.host_target
            case None:
                target = self.config.platform.target
                if target is None:
                    raise Exception('`platform.target` is unconfigured')

        if '/' in target or '.' in target:
            target_name = Path(target).stem
        else:
            target_name = target

        match crate_type:
            case 'bin':
                if not module:
                    module = output / 'main.rs'
                output = Path(target_name) / output.with_stem()
            case _:
                if not module:
                    module = output / 'lib.rs'
                output = Path(target_name) / output.with_name(f"lib{output.stem}.rlib")

        if sysroot:
            module = self.rust_src / module

        crate = Crate(
            name,
            module,
            output,
            crate_type,
            target=target,
            deps=deps,
            builtin=builtin,
        )
        self.crates[crate.name] = crate
        if builtin:
            self.builtins.add(crate)
        return crate
