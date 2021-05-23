#!/usr/bin/python3
import subprocess as sp

if __name__ == '__main__':
    pkgs = [
        'htop',
        'konsole',
        'fonts-inconsolata',
        'kdiff3',
    ]
    cmd = ['sudo', 'apt-get', 'install', '-y'] + pkgs
    print(cmd)
    sp.run(cmd).check_returncode()

