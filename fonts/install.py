#!/usr/bin/env python3
import subprocess as sp

if __name__ == '__main__':
    sp.check_call(['fc-cache', '-fv'])
