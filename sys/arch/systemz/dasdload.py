#!/usr/bin/env python
from pathlib import Path
import argparse
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('output', help='volume output path')
parser.add_argument('--ipl-obj', help='volume configuration file path')
args = parser.parse_args()

dasd_path = Path(args.output)
dasd_path.unlink(missing_ok=True)

with tempfile.TemporaryDirectory() as path:
    path = Path(path)
    cfg_path = path / 'dasd.cfg'

    cfg_path.write_text(f"""IPLVOL 3390-1 * {args.ipl_obj}

SYS1.VTOC       VTOC    CYL 8
SYS1.PARMLIB    EMPTY   CYL 12 12 64 PO FB 80 17600
""")

    result = subprocess.run(['dasdload64', '-z', cfg_path, args.output])
    if result.returncode != 0:
        exit(result.returncode)
