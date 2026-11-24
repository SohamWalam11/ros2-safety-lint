"""rosfix: static checker for silent ROS 2 misconfigurations.

The analysis is done by the bundled native `rosfix` executable; this package only
locates it (for `python -m rosfix` and the `ros2 rosfix` command).
"""

import os
import sys
import sysconfig


def find_rosfix_bin() -> str:
    """Return the path of the bundled rosfix executable."""
    exe = "rosfix" + (".exe" if sys.platform == "win32" else "")
    candidates = [
        os.path.join(sysconfig.get_path("scripts"), exe),
        os.path.join(sysconfig.get_path("scripts", vars={"base": sys.base_prefix}), exe),
    ]
    user_scheme = {"nt": "nt_user", "posix": "posix_user"}.get(os.name)
    if user_scheme:
        try:
            candidates.append(os.path.join(sysconfig.get_path("scripts", scheme=user_scheme), exe))
        except KeyError:
            pass
    for path in candidates:
        if os.path.isfile(path):
            return path
    from shutil import which

    found = which("rosfix")
    if found:
        return found
    raise FileNotFoundError("the rosfix executable was not found next to this Python installation")
