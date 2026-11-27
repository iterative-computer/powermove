"""Unprivileged Darwin process-family accounting; no RSS substitution or sudo."""
import ctypes as C
import os
import platform

FIELDS = """user_time system_time pkg_idle_wkups interrupt_wkups pageins wired_size
resident_size phys_footprint proc_start_abstime proc_exit_abstime child_user_time
child_system_time child_pkg_idle_wkups child_interrupt_wkups child_pageins
child_elapsed_abstime diskio_bytesread diskio_byteswritten cpu_time_qos_default
cpu_time_qos_maintenance cpu_time_qos_background cpu_time_qos_utility
cpu_time_qos_legacy cpu_time_qos_user_initiated cpu_time_qos_user_interactive
billed_system_time serviced_system_time logical_writes lifetime_max_phys_footprint
instructions cycles billed_energy serviced_energy interval_max_phys_footprint runnable_time""".split()


class Usage(C.Structure):
    _fields_ = [("uuid", C.c_uint8 * 16)] + [(name, C.c_uint64) for name in FIELDS]


class Processes:
    def __init__(self):
        if platform.system() != "Darwin":
            raise RuntimeError("This runner requires macOS libproc")
        self.lib = C.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.lib.proc_pid_rusage.argtypes = [C.c_int, C.c_int, C.c_void_p]
        self.lib.proc_listallpids.argtypes = [C.c_void_p, C.c_int]
        self.lib.proc_listchildpids.argtypes = [C.c_int, C.c_void_p, C.c_int]
        self.lib.proc_pidpath.argtypes = [C.c_int, C.c_void_p, C.c_uint32]
        class Timebase(C.Structure):
            _fields_ = [('numer', C.c_uint32), ('denom', C.c_uint32)]
        timebase = Timebase()
        system = C.CDLL('/usr/lib/libSystem.B.dylib')
        if system.mach_timebase_info(C.byref(timebase)) or not timebase.denom:
            raise RuntimeError('Cannot convert Mach CPU ticks to nanoseconds')
        self.timebase = (timebase.numer, timebase.denom)

    def usage(self, pid):
        row = Usage()
        if self.lib.proc_pid_rusage(pid, 4, C.byref(row)):
            raise OSError(C.get_errno(), "proc_pid_rusage denied or process exited")
        result = {key: int(getattr(row, key)) for key in FIELDS}
        # rusage_info reports CPU in Mach absolute ticks, NOT nanoseconds on arm64.
        for key in ['user_time', 'system_time']:
            result[key] = result[key] * self.timebase[0] // self.timebase[1]
        return result

    def all(self):
        size = self.lib.proc_listallpids(None, 0) + 1024
        buf = (C.c_int * size)()
        count = self.lib.proc_listallpids(buf, C.sizeof(buf))
        if count < 0:
            raise OSError("Cannot enumerate processes")
        return list(buf[:count])

    def path(self, pid):
        buf = C.create_string_buffer(4096)
        return buf.value.decode(errors="replace") if self.lib.proc_pidpath(pid, buf, len(buf)) > 0 else ""

    def family(self, pid):
        pending, found = [pid], set()
        while pending:
            parent = pending.pop()
            if parent in found:
                continue
            found.add(parent)
            buf = (C.c_int * 4096)()
            count = self.lib.proc_listchildpids(parent, buf, C.sizeof(buf))
            if count < 0 or count >= len(buf):
                raise RuntimeError("Cannot enumerate complete process family")
            # Unlike proc_listpids, this convenience API returns a PID COUNT.
            pending.extend(p for p in buf[:count] if p > 0)
        return {p: self.usage(p) for p in found}

    def other_apps(self):
        # Do not stop or inspect a real profile. Refuse a shared timing window instead.
        return [pid for pid in self.all() if (
            os.path.basename(self.path(pid)).lower() in {"powermove-native", "powermove", "electron"}
        )]


def delta(before, after, seconds):
    if seconds <= 0 or before.keys() != after.keys():
        raise RuntimeError("Process family changed during accounting window")
    cpu, wakeups = 0, 0
    for pid, end in after.items():
        start = before[pid]
        if start["proc_start_abstime"] != end["proc_start_abstime"]:
            raise RuntimeError("PID was reused")
        cpu += end["user_time"] + end["system_time"] - start["user_time"] - start["system_time"]
        # These are different kernel counters; report separately, never sum as 'wakeups'.
        wakeups += end["interrupt_wkups"] - start["interrupt_wkups"]
    if cpu < 0 or wakeups < 0:
        raise RuntimeError("Non-monotonic process counters")
    return {"cpu_percent": cpu / 1e9 / seconds * 100, "cpu_seconds": cpu / 1e9,
            "interrupt_wakeups_s": wakeups / seconds,
            "package_idle_wakeups_s": sum(after[p]["pkg_idle_wkups"] - before[p]["pkg_idle_wkups"] for p in before) / seconds,
            "family_phys_footprint_mb": sum(p["phys_footprint"] for p in after.values()) / 1e6,
            "processes": len(after)}


def host_seconds():
    """Quartz host clock shared by NSEvent/presentedTime (not Python 3.9's origin)."""
    quartz = C.CDLL('/System/Library/Frameworks/QuartzCore.framework/QuartzCore')
    clock = quartz.CACurrentMediaTime
    clock.restype = C.c_double
    return clock()
