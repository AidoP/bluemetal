#!/usr/bin/env python
from pathlib import Path
import sys
import os

if __name__ == '__main__':
    root = Path(__file__).parent
    os.chdir(root)
    sys.path.insert(0, str(root / 'src'))
    from bs import main
    exit(main(sys.argv))
