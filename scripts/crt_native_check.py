#!/usr/bin/env python3
"""PE-machine helpers for the arm64 CRT staging step (Windows CI only).

This script is invoked by the "Ensure arm64 VC++ CRT redist" workflow step
in an ARM64-NATIVE process (the step selects the toolcache arm64 python
and verifies its PE machine is 0xAA64 before invoking us). That matters:
on the windows-11-arm image the x64-hosted pwsh sees a fusion-redirected
System32 - vcruntime140.dll reads as 0xAA64 (arm64, the only copy) while
vcruntime140_1.dll in the SAME directory reads as 0x8664 (an x64 shadow
copy, present because the x64 redist is also installed). A native
process sees the true files.

Modes:
  check <dir>   report the PE machine of <dir>\vcruntime140_1.dll;
                exit 0 iff it is 0xAA64 (ARM64)
  stage <dir>   verify BOTH VC++ redist DLLs are 0xAA64 in the native
                System32 view, copy them into <dir>, re-verify the copies;
                exit 0 on success
"""
import shutil
import struct
import sys

ARM64 = 0xAA64
SYSTEM32 = r"C:\Windows\System32"
REDIST_DLLS = ("vcruntime140.dll", "vcruntime140_1.dll")


def pe_machine(path):
    """Return the PE machine field, or None if the file is not a PE."""
    with open(path, "rb") as f:
        head = f.read(4096)
    if len(head) < 4096 or head[:2] != b"MZ":
        return None
    off = struct.unpack_from("<I", head, 0x3C)[0]
    if off + 6 > len(head) or head[off:off + 2] != b"PE":
        return None
    return struct.unpack_from("<H", head, off + 4)[0]


def main(argv):
    if len(argv) != 3:
        print("usage: crt_native_check.py <check|stage> <dir>")
        return 2
    mode, arg = argv[1], argv[2]
    if mode == "check":
        m = pe_machine(arg + r"\vcruntime140_1.dll")
        print("MACHINE 0x%X" % m if m is not None else "MACHINE not-a-PE")
        return 0 if m == ARM64 else 1
    if mode == "stage":
        for name in REDIST_DLLS:
            src = SYSTEM32 + "\\" + name
            m = pe_machine(src)
            if m != ARM64:
                print("NATIVE_VIEW_FAIL %s 0x%X" % (name, m or 0))
                return 1
            dst = arg + "\\" + name
            shutil.copyfile(src, dst)
            m2 = pe_machine(dst)
            if m2 != ARM64:
                print("COPY_CHECK_FAIL %s 0x%X" % (name, m2 or 0))
                return 1
            print("STAGED %s 0xAA64" % name)
        print("NATIVE_VIEW_OK")
        return 0
    print("unknown mode: " + mode)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
