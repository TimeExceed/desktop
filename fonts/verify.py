#!/usr/bin/env python3
import subprocess as sp
import re
from sys import stderr
import sys

MAPLE_PAT = re.compile(rb'/maple-mono/')
LXGW_PAT = re.compile(rb'/lxgw-wenkai')

if __name__ == '__main__':
    out = sp.check_output(['fc-list'])
    
    if MAPLE_PAT.search(out) is None:
        print('maple-mono is not installed.', file=stderr)
        maple_ok = False
    else:
        maple_ok = True

    if LXGW_PAT.search(out) is None:
        print('霞鹜文楷 is not installed.', file=stderr)
        lxgw_ok = False
    else:
        lxgw_ok = True
    
    if maple_ok and lxgw_ok:
        sys.exit(0)
    else:
        sys.exit(1)
