#!/usr/bin/env python3
"""验证 /etc/fstab 中数据盘条目是否已正确写入。

只读检查：返回 0 表示 install 已完成，恢复器据此跳过安装。
"""
import json
import subprocess as sp
import sys

MOUNT_POINT = "/media/data"
MIN_SIZE_BYTES = 100 * 1024**3
FSTAB = "/etc/fstab"
SYSTEM_MOUNTS = {"/", "/boot", "/boot/efi"}


def partitions():
    out = sp.check_output([
        "lsblk", "--json", "-b",
        "-o", "NAME,SIZE,TYPE,FSTYPE,UUID,MOUNTPOINTS",
    ])
    result = []
    for disk in json.loads(out)["blockdevices"]:
        for part in disk.get("children") or []:
            if part.get("type") == "part":
                result.append(part)
    return result


def is_candidate(part):
    if (part.get("fstype") or "") in ("", "swap"):
        return False
    if not part.get("uuid"):
        return False
    if int(part.get("size") or 0) < MIN_SIZE_BYTES:
        return False
    mounts = {m for m in (part.get("mountpoints") or []) if m}
    return not (mounts & SYSTEM_MOUNTS)


def fstab_options(fstype):
    if fstype == "btrfs":
        return "defaults,noatime,compress=zstd"
    else:
        return "defaults,noatime"


def find_entry(mount_point):
    with open(FSTAB, encoding="utf-8") as f:
        for line in f:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            fields = stripped.split()
            if len(fields) >= 2 and fields[1] == mount_point:
                return fields
    return None


def main():
    candidates = [p for p in partitions() if is_candidate(p)]
    if len(candidates) != 1:
        print(
            f"无法唯一确定数据盘：{len(candidates)} 个候选",
            file=sys.stderr,
        )
        return 1

    part = candidates[0]
    src = "UUID=" + part["uuid"]
    expected = [src, MOUNT_POINT, part["fstype"], fstab_options(part["fstype"])]

    entry = find_entry(MOUNT_POINT)
    if entry is None:
        print(f"fstab 中缺少 {MOUNT_POINT} 条目", file=sys.stderr)
        return 1

    if entry[:4] != expected:
        print(
            f"fstab 条目不符合预期：{' '.join(entry)}",
            file=sys.stderr,
        )
        return 1

    print(f"fstab 条目正确：{' '.join(entry)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
