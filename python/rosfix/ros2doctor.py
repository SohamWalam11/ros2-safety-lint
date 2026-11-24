"""`ros2 doctor` integration: rosfix's environment checks as a ros2doctor check and report.

ros2doctor loads these classes through the `ros2doctor.checks` and `ros2doctor.report` entry
points of this package, so after `pip install rosfix`, `ros2 doctor` also checks the
middleware environment (RMW_IMPLEMENTATION, ROS_DOMAIN_ID, discovery settings, DDS profile
files) and `ros2 doctor --report` lists rosfix's findings. The checks themselves run in the
native `rosfix doctor` command; this module only translates its JSON output.
"""

import json
import subprocess

from ros2doctor.api import DoctorCheck, DoctorReport, Report, Result
from ros2doctor.api.format import doctor_error, doctor_warn

from rosfix import find_rosfix_bin

CATEGORY = 'rosfix'


def _checks():
    """Run `rosfix doctor --format json`; None if rosfix cannot be run."""
    try:
        out = subprocess.run(
            [find_rosfix_bin(), 'doctor', '--format', 'json'],
            capture_output=True, text=True, timeout=60, check=False)
        return json.loads(out.stdout)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def _line(c):
    text = f"{c['subject']}: {c['message']}"
    if c.get('help'):
        text += f" ({c['help']})"
    return text


class RosfixEnvironmentCheck(DoctorCheck):
    """Middleware environment checks from rosfix (see `rosfix docs doctor`)."""

    def category(self):
        return CATEGORY

    def check(self):
        result = Result()
        checks = _checks()
        if checks is None:
            doctor_warn('rosfix: could not run `rosfix doctor`')
            result.add_warning()
            return result
        for c in checks:
            if c['level'] == 'error':
                doctor_error(f'rosfix: {_line(c)}')
                result.add_error()
            elif c['level'] == 'warning':
                doctor_warn(f'rosfix: {_line(c)}')
                result.add_warning()
        return result


class RosfixEnvironmentReport(DoctorReport):
    """`ros2 doctor --report` section listing every rosfix environment check."""

    def category(self):
        return CATEGORY

    def report(self):
        report = Report('ROSFIX ENVIRONMENT')
        for c in _checks() or []:
            report.add_to_report(c['subject'], f"{c['level']}: {c['message']}")
        return report
