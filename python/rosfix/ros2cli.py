"""`ros2 rosfix …`: a ros2cli command that forwards to the rosfix executable.

Registered through the `ros2cli.command` entry point; only imported when ros2cli runs,
so this module may depend on it.
"""

import argparse
import subprocess

try:
    from ros2cli.command import CommandExtension
except ImportError:  # ros2cli not installed: provide a stub so imports still work
    class CommandExtension:  # type: ignore[no-redef]
        pass

from rosfix import find_rosfix_bin


class RosfixCommand(CommandExtension):
    """Check a ROS 2 workspace for silent misconfigurations."""

    def add_arguments(self, parser, cli_name):
        parser.add_argument(
            "rosfix_args",
            nargs=argparse.REMAINDER,
            help="arguments passed to rosfix (e.g. `src --format sarif`); see `ros2 rosfix -- --help`",
        )

    def main(self, *, parser, args):
        forwarded = [a for a in args.rosfix_args if a != "--"]
        return subprocess.run([find_rosfix_bin(), *forwarded]).returncode
