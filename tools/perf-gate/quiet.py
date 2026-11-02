"""Portable condition policy; Darwin collection is injected for Linux tests."""

import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess
import threading


def finite(value):
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
    )


def foreign_apps(apps, owned_executables, owned_groups, getpgid=os.getpgid):
    def measured(app):
        try:
            return (
                str(Path(app["executable"]).resolve()) in owned_executables
                and getpgid(app["pid"]) in owned_groups
            )
        except ProcessLookupError:
            return False

    return sorted(a["id"] for a in apps if not measured(a))


def blockers(state, *, hz, baseline=None, allowed_apps=(), max_load_per_cpu=0.5):
    """Unknown observations fail closed. Never stop another app to make it quiet.

    Load is checked at all three horizons, relative to logical CPUs. A noisy
    observation rejects the entire pair, including its otherwise faster side.
    """
    if not isinstance(state, dict):
        return ["condition snapshot unavailable"]
    reasons = []
    cpus, load = state.get("logical_cpus"), state.get("load")
    if (
        not finite(cpus)
        or cpus < 1
        or not isinstance(load, list)
        or len(load) != 3
        or any(not finite(x) or x < 0 for x in load)
    ):
        reasons.append("load unavailable")
    elif max(load) / cpus > max_load_per_cpu:
        reasons.append("load above quiet threshold")
    if state.get("thermal") != "nominal":
        reasons.append("thermal state is not nominal or unavailable")
    if state.get("ac_power") is not True:
        reasons.append("AC power unavailable")
    if state.get("screen_unlocked") is not True:
        reasons.append("unlocked session unavailable")
    if not finite(state.get("free_bytes")) or state["free_bytes"] < 8 * 1024**3:
        reasons.append("less than 8 GiB free or disk unavailable")
    apps = state.get("other_apps")
    if not isinstance(apps, list) or any(not isinstance(a, str) for a in apps):
        reasons.append("application inventory unavailable")
    elif set(apps) - set(allowed_apps):
        reasons.append("other GUI applications active")
    if state.get("active_builds") != []:
        reasons.append("active builds or process inventory unavailable")
    displays = state.get("displays")
    if (
        not isinstance(displays, list)
        or len(displays) != 1
        or not isinstance(displays[0], dict)
        or not finite(displays[0].get("hz"))
        or abs(displays[0]["hz"] - hz) > 0.5
    ):
        reasons.append("one stable display at requested refresh required")
    if not isinstance(state.get("os_build"), str) or not state["os_build"]:
        reasons.append("OS build unavailable")
    if baseline:
        for field in ("displays", "os_build", "logical_cpus"):
            if state.get(field) != baseline.get(field):
                reasons.append(field + " changed during pair")
    return reasons


class MacProbe:
    def __init__(self, scratch, owned_executables=()):
        if platform.system() != "Darwin":
            raise RuntimeError(
                "Live gates require macOS; use --replay for Linux fixtures"
            )
        from driver import Processes

        self.proc = Processes()
        self.owned = {str(Path(p).resolve()) for p in owned_executables}
        self.groups = set()
        self.group_lock = threading.Lock()
        self.binary = Path(scratch) / "conditions-probe"
        # Compile before quiet checks, never during a measurement interval.
        subprocess.run(
            [
                "swiftc",
                "-O",
                str(Path(__file__).with_name("conditions.swift")),
                "-o",
                str(self.binary),
            ],
            check=True,
            capture_output=True,
            timeout=120,
        )

    def own_group(self, pid):
        with self.group_lock:
            self.groups.add(pid)

    def release_group(self, pid):
        with self.group_lock:
            self.groups.discard(pid)

    def sample(self):
        from driver import command, power, screen_unlocked

        result = subprocess.run(
            [str(self.binary)], capture_output=True, text=True, check=True, timeout=10
        )
        state = json.loads(result.stdout)
        # An existing copy of the configured app is still a foreign app. Exempt
        # only the measured executable in this runner's active process group.
        with self.group_lock:
            groups = set(self.groups)

        state["other_apps"] = foreign_apps(state["apps"], self.owned, groups)
        del state["apps"]
        builds = []
        for pid in self.proc.all():
            name = Path(self.proc.path(pid)).name
            if name in {
                "rustc",
                "cargo",
                "clang",
                "clang++",
                "swift-frontend",
                "ld",
                "xcodebuild",
            }:
                builds.append(name)
        state.update(
            load=list(os.getloadavg()),
            logical_cpus=os.cpu_count(),
            ac_power=power(),
            screen_unlocked=screen_unlocked(),
            free_bytes=shutil.disk_usage("/").free,
            active_builds=sorted(builds),
            os_build=command(["sw_vers", "-buildVersion"]).stdout.strip(),
        )
        return state
