#!/usr/bin/env python3
import subprocess as sp

if __name__ == '__main__':
    sp.check_call([
        'dnf',
        '-y',
        'install', 
        '@multimedia', 
        '--setopt=install_weak_deps=False',
        '--exclude=PackageKit-gstreamer-plugin',
    ])

    if sp.call(['rpm', '-q', 'ffmpeg']) != 0:
        sp.check_call([
            'dnf', 
            '-y',
            'swap', 
            'ffmpeg-free', 
            'ffmpeg',
            '--allowerasing',
        ])

    sp.check_call(['dnf', '-y', 'install', '@vlc'])
