#!/usr/bin/python3
import subprocess as sp
from pathlib import Path
import hashlib
from collections import deque

def digest(fn):
    m = hashlib.sha256()
    with fn.open('rb') as fp:
        m.update(fp.read())
    return m.digest()

def diff(src, des):
    if src.exists():
        m_src = digest(src)
    else:
        m_src = None
    if des.exists():
        m_des = digest(des)
    else:
        m_des = None
    return m_src != m_des

def update_apt_src(src, des_d):
    should_update = diff(src, des_d.joinpath(src.name))
    if should_update:
        if src.exists():
            cmd = [
                'sudo', 'install', '-o', 'root', '-g', 'root', '-m', '644', 
                '%s' % src,
                '%s' % des_d,
            ]
            print(cmd)
            sp.run(cmd).check_returncode()
        else:
            cmd = [
                'sudo', 'rm', '-f', '%s' % des_d.joinpath(src.name),
            ]
            check_run_cmd(cmd)
    return should_update

def install_apt_source(src, gpg):
    gpg_d = Path('/etc/apt/trusted.gpg.d/')
    src_d = Path('/etc/apt/sources.list.d/')
    gpg_updated = update_apt_src(gpg, gpg_d)
    src_updated = update_apt_src(src, src_d)
    return gpg_updated or src_updated

def check_run_cmd(cmd):
    print(cmd)
    sp.run(cmd).check_returncode()

def install_pkgs():
    pkgs = [
        'htop',
        'konsole',
        'fonts-inconsolata',
        'kdiff3',
        'code',
        'git-gui', 'gitk',
        'docker.io',
        'exa',
        'python3-pip',
        'python3-pygments',
        'emacs',
        'apt-transport-https',
        'aptitude',
        'ipython3',
        'tmux',
        'flameshot',
        'vulkan-tools',
        'gnome-control-center', 'gnome-system-tools',
        'xfce4-notes', 'xfce4-notes-plugin', 'xfce4-systemload-plugin', 'xfce4-netload-plugin', 'xfce4-goodies', 'xfce4-cpugraph-plugin',
        'net-tools',
        'fcitx5', 'fcitx5-pinyin', 'fcitx5-chinese-addons',
    ]
    cmd = ['sudo', 'apt-get', 'install', '-y'] + pkgs
    check_run_cmd(cmd)

def disable_sysctl_units():
    units = ['apt-daily-upgrade.timer', 'apt-daily.timer']
    for x in units:
        check_run_cmd(['sudo', 'systemctl', 'disable'] + [x])

def walk_files(root):
    q = deque([root])
    while len(q) > 0:
        d = q.popleft()
        for x in d.iterdir():
            if x.is_dir():
                q.append(x)
            elif x.is_file():
                yield x.relative_to(root)

def prepare_home():
    home = Path.home()
    src_rt = Path('home.cfg')
    install = ['install', '-m', '644']
    for x in walk_files(src_rt):
        src = src_rt.joinpath(x)
        dst = home.joinpath(x)
        dst.parent.mkdir(parents=True, exist_ok=True)
        cmd = install + [str(src.resolve()), str(dst.resolve())]
        check_run_cmd(cmd)

def prepare_etc():
    etc = Path('/etc')
    src_rt = Path('etc')
    install = ['sudo', 'install', '-o', 'root', '-g', 'root', '-m', '644']
    for x in walk_files(src_rt):
        src = src_rt.joinpath(x)
        dst = etc.joinpath(x)
        dst.parent.mkdir(parents=True, exist_ok=True)
        cmd = install + [str(src.resolve()), str(dst.resolve())]
        check_run_cmd(cmd)

if __name__ == '__main__':
    src_updated = install_apt_source(Path('vscode.list'), Path('packages.microsoft.gpg'))
    if src_updated:
        cmd = [
            'sudo', 'apt-get', 'update',
        ]
        check_run_cmd(cmd)
    install_pkgs()
    disable_sysctl_units()
    prepare_home()
    prepare_etc()
