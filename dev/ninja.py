"""
Ninja file generation.
"""

from typing import TextIO
from pathlib import Path


def _make_list(
    values: list[str | Path] | str | Path| None,
    prefix: str = '',
) -> str:
    if values is None:
        return ''
    if isinstance(values, Path):
        return prefix + str(values)
    if isinstance(values, str):
        return prefix + values
    values = [str(v) for v in values]
    if len(values) == 0:
        return ''
    return prefix + ' '.join(values)


class Writer:
    file: TextIO

    def __init__(self, file: TextIO):
        self.file = file


    def _write(self, line: str):
        self.file.write(line + '\n')


    def variable(
        self,
        name: str,
        value: str,
        /,
        indent: int = 0,
    ):
        self._write(f"{'  ' * indent}{name} = {value}")


    def rule(
        self,
        name: str,
        /,
        *command,
        depfile: str | None = None,
        **variables: list[str] | str,
    ):
        self._write(f"rule {name}")
        self.variable('command', ' '.join(str(s) for s in command), indent=1)
        if depfile is not None:
            self.variable('deps', 'gcc', indent=1)
            self.variable('depfile', depfile, indent=1)
        for k, v in variables.items():
            self.variable(k, _make_list(v), indent=1)

    def build(
        self,
        rule: str,
        *,
        outputs: list[str],
        inputs: list[str] | None = None,
        dependencies: list[str | Path] | None = None,
        **variables: list[str] | str | None,
    ):
        self._write(f"build {_make_list(outputs)}: {rule}{_make_list(inputs, prefix=' ')}{_make_list(dependencies, prefix=' | ')}")
        for k, v in variables.items():
            if v is None:
                continue
            if not isinstance(v, str):
                v = [str(i) for i in v]
                if len(v) == 0:
                    continue
                v = ' '.join(v)
            self.variable(k, v, indent=1)

    def subninja(
        self,
        path: str | Path,
        /,
        ):
        self._write(f"subninja {path}")
