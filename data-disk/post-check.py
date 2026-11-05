#!/usr/bin/env python3
"""重启后检查：/media/data 是否已由 fstab 自动挂载。

本步骤不安装任何东西，只做检查——fstab 写对了、重启后挂上了，
才算真的恢复完成。
"""
import subprocess as sp
import sys

MOUNT_POINT = "/media/data"


def main():
    if sp.call(["findmnt", MOUNT_POINT]) != 0:
        print(f"{MOUNT_POINT} 未挂载，fstab 可能未生效", file=sys.stderr)
        return 1

    out = sp.check_output(
        ["findmnt", "-no", "SOURCE,FSTYPE", MOUNT_POINT],
        text=True,
    ).strip()
    print(f"{MOUNT_POINT} 已挂载：{out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
