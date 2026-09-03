from pathlib import Path
import subprocess
import sys


def main() -> int:
    print('unimplemented')
    return 0


def run(platform: str) -> int:
    match platform:

        case 'systemz':
            target = 'build/systemz/iplvol.3390'
        case _:
            target = 'build/platform/' + platform + '/nucleus'


    result = subprocess.run(['ninja', target])
    if result.returncode != 0:
        return result.returncode

    match platform:
        case 'systemz':
            return run_hercules()
        case _ if '/' in platform:
            system, machine = platform.split('/', maxsplit=1)
            return run_qemu_basic(system, machine, target)

    print('unknown platform', file=sys.stderr)
    return 128


def run_qemu_basic(system: str, machine: str, target: str) -> int:
    try:
        result = subprocess.run(
            [f"qemu-system-{system}", '-machine', machine, '-display', 'none', '-serial', 'stdio', '-kernel', target],
            check=False,
        )
    except KeyboardInterrupt:
        return -1
    if result.returncode != 0:
        return result.returncode


def run_hercules() -> int:
    path = Path('run/hercules')
    path.mkdir(exist_ok=True, parents=True)

    (path / 'hercules.rc').write_text('ipl 1')
    (path / 'hercules.cnf').write_text("""
ARCHLVL z/Arch
CODEPAGE 819/1047
MAINSIZE 64M
NUMCPU 2

# IPL Volumes
0001 3390       ../../build/systemz/iplvol.3390

# Consoles
1000 3215-C     /
""")

    result = subprocess.run(['hercules'], cwd=path)
    if result.returncode != 0:
        return result.returncode
