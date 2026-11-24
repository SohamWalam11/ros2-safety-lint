"""`python -m rosfix …` runs the bundled executable."""

import os
import subprocess
import sys

from rosfix import find_rosfix_bin


def main() -> int:
    exe = find_rosfix_bin()
    if sys.platform == "win32":
        return subprocess.run([exe, *sys.argv[1:]]).returncode
    os.execv(exe, [exe, *sys.argv[1:]])
    return 0  # unreachable


if __name__ == "__main__":
    sys.exit(main())
