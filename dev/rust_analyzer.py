from pathlib import Path
from typing import Literal
import json


class Runnable:
    def __init__(
        self,
        program: str,
        /,
        *args: str | Path,
        cwd: str = '',
        kind: Literal['testOne', 'flycheck', 'run'],
    ):
        self.program = program
        self.args = [str(arg) for arg in args]
        self.cwd = cwd
        self.kind = kind


    def as_dict(self, project: 'Project') -> dict:
        return {
            'program': self.program,
            'args': self.args,
            'cwd': self.cwd,
            'kind': self.kind,
        }


class BuildInfo:
    def __init__(
        self,
        label: str | Path,
        path: str | Path,
        kind: Literal['bin', 'lib', 'test'],
    ):
        self.label = str(label) + '.flycheck'
        self.path = str(path)
        self.kind = kind


    def as_dict(self, project: 'Project') -> dict:
        return {
            'label': self.label,
            'build_file': self.path,
            'target_kind': self.kind,
        }


class Crate:
    def __init__(
        self,
        path: Path | str,
        *,
        index: int,
        edition: str,
        name: str | None = None,
        version: str | None = None,
        target: str | None = None,
        deps: list[int] = [],
        build_info: BuildInfo | None = None,
    ):
        self.path = str(path)
        self.name = name
        self.version = version
        self.edition = edition
        self.target = target
        self.index = index
        self.deps = deps
        self.build_info = build_info


    def as_dict(self, project: 'Project') -> dict:
        deps = []
        for dep in self.deps:
            crate = project.crates[project.crate_names[dep]]
            deps.append({ 'crate': crate.index, 'name': crate.name })
        raw = {
            'root_module': self.path,
            'edition': self.edition,
            'deps': deps,
        }
        if self.name is not None:
            raw['display_name'] = self.name
        if self.version is not None:
            raw['version'] = self.version
        if self.target is not None:
            raw['target'] = self.target
        if self.build_info:
            raw['build'] = self.build_info.as_dict(project)
        return raw


class Project:
    def __init__(
        self,
        *,
        sysroot: str | Path | None = None,
    ):
        self.sysroot = str(sysroot) if sysroot else None
        self.crates = []
        self.crate_names = {}
        self.runnables = []

    def crate(
        self,
        path: Path,
        name: str,
        edition: str,
        deps: set() = set(),
        build_info: BuildInfo | None = None,
    ):
        crate = Crate(
            path,
            name=name,
            edition=edition,
            index=len(self.crates),
            deps=deps,
            build_info=build_info,
        )
        self.crates.append(crate)
        self.crate_names[crate.name] = crate.index


    def runnable(
        self,
        program: str,
        *args: str,
        cwd: str = '',
        kind: Literal['testOne', 'flycheck', 'run'],
    ):
        self.runnables.append(Runnable(
            program,
            *args,
            cwd=cwd,
            kind=kind,
        ))


    def as_dict(self) -> dict:
        raw = {
            'crates': [c.as_dict(self) for c in self.crates],
        }
        if self.sysroot is not None:
            raw['sysroot'] = str(self.sysroot)
        if len(self.runnables) > 0:
            raw['runnables'] = [r.as_dict(self) for r in self.runnables]
        return raw


    def write(self, path: str | Path):
        with open(path, 'w') as f:
            json.dump(self.as_dict(), f, indent=4)
