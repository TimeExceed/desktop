#!/usr/bin/env python3
import subprocess as sp
import re
from sys import stderr
import sys

PATTERN = re.compile(rb'Installed\s*:\s*yes')

def check_group_installed(g: str) -> None:
    out = sp.check_output(['dnf', 'group', 'info', g])
    if PATTERN.search(out) is None:
        print(f'@{g} is not installed.', file=stderr)
        sys.exit(1)
    else:
        print(f'@{g} is installed.', file=stderr)


if __name__ == '__main__':
    check_group_installed('multimedia')
    sp.check_call(['rpm', '-qv', 'ffmpeg'])
    check_group_installed('vlc')
