from pathlib import Path

class Platform:
    target: str | None = None

    def load(self, raw: dict):
        if 'target' in raw:
            self.target = raw.pop('target')
        if len(raw) > 0:
            raise Exception(f"unexpected keys in `platform`: {', '.join(raw.keys())}")


class Config:
    includes: list[Path]

    def __init__(self):
        self.includes = []
        self.platform = Platform()

    def load(self, path: Path):
        import tomllib
        with open(path, 'rb') as f:
            raw = tomllib.load(f)

        for include in raw.get('include', []):
            p = Path(include)
            parent = path.parent
            if parent is not None:
                p = parent / p
            self.load(p)
            self.includes.append(p)

        if platform := raw.get('platform'):
            self.platform.load(platform)
