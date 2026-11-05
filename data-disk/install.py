#!/usr/bin/env python3
"""探测数据盘，写入 /etc/fstab 并挂载。

恢复器视角：把本机上一块容量足够、且不承载系统挂载点的分区，
固定为 fstab 中的 /media/data 条目。
"""
import json
import os
import subprocess as sp
import sys

MOUNT_POINT = "/media/data"
MIN_SIZE_BYTES = 100 * 1024**3
FSTAB = "/etc/fstab"
SYSTEM_MOUNTS = {"/", "/boot", "/boot/efi"}

def partitions():
    """返回所有分区（含 fstype / uuid / size / mountpoints）。"""
    out = sp.check_output([
        "lsblk", "--json", "-b",
        "-o", "NAME,SIZE,TYPE,FSTYPE,UUID,MOUNTPOINTS",
    ])
    result = []
    for disk in json.loads(out).get("blockdevices", []):
        for part in disk.get("children", []):
            if part.get("type") == "part":
                result.append(part)
    return result


def is_candidate(part):
    """容量 ≥ 阈值、有 UUID、非 swap、不承载系统挂载点。"""
    if (part.get("fstype") or "") in ("", "swap"):
        return False
    if not part.get("uuid"):
        return False
    if int(part.get("size") or 0) < MIN_SIZE_BYTES:
        return False
    mounts = {m for m in part.get("mountpoints", []) if m}
    return not (mounts & SYSTEM_MOUNTS)


def fstab_options(fstype):
    if fstype == "btrfs":
        return "defaults,noatime,compress=zstd"
    else:
        return "defaults,noatime"


def read_fstab(mount_point):
    with open(FSTAB, encoding="utf-8") as fp:
        for line in fp:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            fields = stripped.split()
            if len(fields) >= 2 and fields[1] == mount_point:
                return fields
    return None


def is_mounted(mount_point):
    return sp.call(["findmnt", mount_point]) == 0

def main():
    candidates = [p for p in partitions() if is_candidate(p)]
    if len(candidates) != 1:
        names = ", ".join("/dev/" + p["name"] for p in candidates) or "无"
        print(
            f"期望恰好一个数据盘候选（≥100G 且非系统分区），实际 {len(candidates)} 个：{names}",
            file=sys.stderr,
        )
        return 1

    part = candidates[0]
    src = "UUID=" + part["uuid"]
    fstype = part["fstype"]
    expected = [src, MOUNT_POINT, fstype, fstab_options(fstype), "0", "2"]

    entry = read_fstab(MOUNT_POINT)
    if entry is not None:
        if entry[:4] != expected[:4]:
            print(
                f"fstab 中 {MOUNT_POINT} 已有不同条目：{' '.join(entry)}",
                file=sys.stderr,
            )
            return 1
        print(f"fstab 条目已正确：{' '.join(entry)}", file=sys.stderr)
    else:
        with open(FSTAB, "a", encoding="utf-8") as f:
            f.write(" ".join(expected) + "\n")
        print(f"已写入 fstab：{' '.join(expected)}", file=sys.stderr)

    os.makedirs(MOUNT_POINT, exist_ok=True)

    if is_mounted(MOUNT_POINT):
        print(f"{MOUNT_POINT} 已挂载", file=sys.stderr)
    else:
        sp.check_call(["mount", MOUNT_POINT])
        print(f"已挂载 {MOUNT_POINT}", file=sys.stderr)

    return 0


if __name__ == "__main__":
    sys.exit(main())
