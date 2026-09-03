from pathlib import Path
import json


class Crate:
    def __init__(
        self,
        path: Path | str,
        *,
        edition: str,
        name: str | None = None,
        version: str | None = None,
        target: str | None = None,
    ):
        self.path = str(path)
        self.name = name
        self.version = version
        self.edition = edition
        self.target = target


    def as_dict(self) -> dict:
        raw = {
            'root_module': self.path,
            'edition': self.edition,
        }
        if self.name is not None:
            raw['display_name'] = self.path
        if self.version is not None:
            raw['version'] = self.version
        if self.target is not None:
            raw['target'] = self.target
        return raw


class Project:
    def __init__(
        self,
        *,
        sysroot: str | Path | None = None,
        crates: list[Crate] = [],
    ):
        self.sysroot = str(sysroot) if sysroot else None
        self.crates = crates


    def as_dict(self) -> dict:
        raw = {
            'crates': [c.as_dict() for c in self.crates],
        }
        if self.sysroot is not None:
            raw['sysroot'] = str(self.sysroot)
        return raw


    def write(self, path: str | Path):
        with open(path, 'w') as f:
            json.dump(self.as_dict(), f)
