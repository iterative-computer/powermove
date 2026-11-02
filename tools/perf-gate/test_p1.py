"""Recorded protocol fixtures exercise P1 without a screen, app or native SDK."""

import copy
from contextlib import nullcontext
import json
from pathlib import Path
import subprocess
import os
import shutil
import sys
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

import collect
import configure
import gate
from budgets import ROWS
from evidence import CONTRACTS, METRICS, outliers, summarize
from quiet import blockers, foreign_apps

HERE = Path(__file__).resolve().parent


def fixture():
    return json.loads((HERE / "fixtures/recorded-p1.json").read_text())


def recording():
    return gate.expand_fixture(fixture())


class PolicyTests(unittest.TestCase):
    def test_every_budget_clause_can_fail_the_row(self):
        cases = {
            "cold_launch": {"latency_ms": 400},
            "open": {"latency_ms": 160, "main_max_ms": 9},
            "scrub_cached": {"own_share_ms": 9.334, "on_screen_ms": 30},
            "scrub_random_seek": {"preview_ms": 11, "full_ms": 151},
            "playback": {
                "drops": 1,
                "frame_work_p95_ms": 8.334,
                "duration_s": 59.99,
                "fps": 30,
            },
            "playback_cost": {"cpu_percent": 13, "gpu_ms": 5, "energy_j": 9},
            "av": {"mean_abs_offset_ms": 1.001, "jitter_p95_ms": 2.001},
            "edit_repaint": {
                "own_share_ms": 9.334,
                "on_screen_ms": 30,
                "main_max_ms": 4.001,
            },
            "idle": {"cpu_percent": 1, "extension_wakeups_s": 2.001},
            "memory_small": {"footprint_mb": 251},
        }
        for row, metrics in cases.items():
            for metric, value in metrics.items():
                data = recording()
                for attempt in data["attempts"]:
                    if attempt["row"] == row:
                        attempt["native"]["metrics"][metric] = value
                self.assertEqual(summarize(data)[row]["status"], "FAIL", (row, metric))

    def test_thirty_recorded_pairs_pass_every_clause(self):
        result = summarize(recording())
        self.assertEqual(set(result), set(ROWS))
        self.assertTrue(all(r["status"] == "PASS" for r in result.values()))
        self.assertTrue(all(len(r["accepted_pairs"]) == 30 for r in result.values()))

    def test_each_endpoint_attestation_is_required_on_both_sides(self):
        for row, contract in CONTRACTS.items():
            for side in ("native", "electron"):
                for key in contract:
                    data = recording()
                    next(a for a in data["attempts"] if a["row"] == row)[side][
                        "evidence"
                    ].pop(key)
                    self.assertEqual(
                        summarize(data)[row]["status"], "INCOMPLETE", (row, side, key)
                    )

    def test_missing_reference_retains_native_numbers_but_has_no_pair_ratio(self):
        data = recording()
        for a in data["attempts"]:
            if a["row"] == "scrub_cached":
                a["electron"]["metrics"].pop("on_screen_ms")
        result = summarize(data)["scrub_cached"]
        self.assertEqual(result["stats"]["native"]["on_screen_ms"]["n"], 30)
        self.assertIsNone(result["paired_stats"]["native"]["on_screen_ms"])
        self.assertEqual(result["status"], "INCOMPLETE")

    def test_disjoint_metric_observations_cannot_form_thirty_pairs(self):
        data = recording()
        pairs = [a for a in data["attempts"] if a["row"] == "scrub_cached"]
        pairs[0]["native"]["metrics"].pop("on_screen_ms")
        pairs[-1]["electron"]["metrics"].pop("on_screen_ms")
        result = summarize(data)["scrub_cached"]
        self.assertEqual(result["paired_stats"]["native"]["on_screen_ms"]["n"], 28)
        self.assertEqual(result["paired_indices"]["on_screen_ms"], list(range(1, 29)))
        self.assertEqual(result["status"], "INCOMPLETE")

    def test_noisy_during_sample_excludes_entire_pair(self):
        data = recording()
        data["attempts"][0]["conditions"][1]["thermal"] = "serious"
        result = summarize(data)["cold_launch"]
        self.assertEqual(result["stats"]["native"]["latency_ms"]["n"], 29)
        self.assertEqual(result["stats"]["electron"]["latency_ms"]["n"], 29)
        self.assertEqual(len(result["rejected_pairs"]), 1)
        self.assertEqual(result["status"], "INCOMPLETE")

    def test_timing_outliers_are_flagged_and_remain_in_tail(self):
        data = recording()
        for a in data["attempts"][28:30]:
            a["native"]["metrics"]["latency_ms"] = 900
        result = summarize(data)["cold_launch"]
        self.assertEqual(result["outliers"]["native"]["latency_ms"], [28, 29])
        self.assertEqual(result["stats"]["native"]["latency_ms"]["n"], 30)
        self.assertEqual(result["stats"]["native"]["latency_ms"]["p95"], 900)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(outliers([1, 2, 3]), [])

    def test_bad_counters_and_duplicate_indices_cannot_pass(self):
        for invalid in (True, None, "0", -1, float("nan"), float("inf")):
            data = recording()
            data["attempts"][0]["native"]["metrics"]["latency_ms"] = invalid
            self.assertEqual(summarize(data)["cold_launch"]["status"], "INCOMPLETE")
        data = recording()
        data["attempts"][1]["index"] = data["attempts"][0]["index"]
        self.assertEqual(summarize(data)["cold_launch"]["status"], "INCOMPLETE")

        data = recording()
        next(a for a in data["attempts"] if a["row"] == "scrub_cached")["native"][
            "evidence"
        ]["exact_cached_frame"] = 1
        self.assertEqual(summarize(data)["scrub_cached"]["status"], "INCOMPLETE")

    def test_noncertified_quality_and_standin_rigs_never_pass(self):
        for settings in (
            {"quality_certified": False},
            {"rig_certified": False},
            {"rig": "stand-in"},
        ):
            data = recording()
            data["context"].update(settings)
            self.assertTrue(
                all(r["status"] == "INCOMPLETE" for r in summarize(data).values())
            )

    def test_every_required_metric_is_required(self):
        for row, metrics in METRICS.items():
            for metric in metrics:
                data = recording()
                next(a for a in data["attempts"] if a["row"] == row)["native"][
                    "metrics"
                ].pop(metric)
                self.assertEqual(
                    summarize(data)[row]["status"], "INCOMPLETE", (row, metric)
                )

    def test_gpu_and_raf_cannot_be_called_physical_presentation(self):
        data = recording()
        for a in data["attempts"]:
            if a["row"] == "edit_repaint":
                a["electron"]["evidence"]["screen_endpoint"] = "gpu-complete-next-raf"
        self.assertEqual(summarize(data)["edit_repaint"]["status"], "INCOMPLETE")

    def test_missing_condition_phase_and_incomplete_pair_are_rejected(self):
        for mutation in ("phase", "side"):
            data = recording()
            if mutation == "phase":
                data["attempts"][0]["conditions"].pop()
            else:
                del data["attempts"][0]["electron"]
            result = summarize(data)["cold_launch"]
            self.assertEqual(result["status"], "INCOMPLETE")
            self.assertEqual(len(result["rejected_pairs"]), 1)


class QuietTests(unittest.TestCase):
    def test_existing_copy_of_configured_executable_remains_foreign(self):
        executable = str(Path(sys.executable).resolve())
        apps = [
            dict(id="measured", executable=executable, pid=10),
            dict(id="existing", executable=executable, pid=20),
            dict(id="other", executable="/synthetic/other", pid=30),
        ]
        self.assertEqual(
            foreign_apps(
                apps, {executable}, {100}, lambda pid: 100 if pid == 10 else 200
            ),
            ["existing", "other"],
        )

    def test_nominal_recorded_condition(self):
        f = fixture()
        self.assertEqual(
            blockers(f["baseline"], hz=120, allowed_apps=f["context"]["allowed_apps"]),
            [],
        )

    def test_all_condition_failures_are_closed(self):
        f = fixture()
        mutations = [
            {"load": [0, 0, 9]},
            {"load": [float("nan"), 0, 0]},
            {"load": None},
            {"thermal": "fair"},
            {"thermal": None},
            {"ac_power": False},
            {"screen_unlocked": None},
            {"free_bytes": 0},
            {"active_builds": ["rustc"]},
            {"active_builds": None},
            {"other_apps": ["com.example.unrelated-editor"]},
            {"other_apps": None},
            {"displays": []},
            {"displays": [{"hz": 60}]},
            {"os_build": ""},
        ]
        for change in mutations:
            state = dict(f["baseline"], **change)
            self.assertTrue(
                blockers(state, hz=120, allowed_apps=f["context"]["allowed_apps"]),
                change,
            )

    def test_display_mode_and_os_cannot_change_after_baseline(self):
        f = fixture()
        for field, value in [
            ("os_build", "different"),
            ("logical_cpus", 4),
            ("displays", [dict(f["baseline"]["displays"][0], mode=2)]),
        ]:
            state = dict(f["baseline"], **{field: value})
            self.assertIn(
                field + " changed during pair",
                blockers(
                    state,
                    hz=120,
                    baseline=f["baseline"],
                    allowed_apps=f["context"]["allowed_apps"],
                ),
            )


class RunnerTests(unittest.TestCase):
    def test_condition_rejection_is_replaced_but_timing_failure_is_not(self):
        f = fixture()
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            config = dict(
                context=f["context"],
                apps={},
                reference="/unused",
                scenarios={},
                max_attempts_per_row=31,
                retry_settle_s=0,
            )

            def measured(config, row, side, attempt, scratch, probe, call):
                attempt[side] = copy.deepcopy(f["rows"][row][side])
                state = dict(f["baseline"])
                if attempt["index"] == 0:
                    state["thermal"] = "fair"
                if row == "cold_launch":
                    attempt[side]["metrics"]["latency_ms"] = 900
                attempt["conditions"].extend(
                    dict(state, phase=side + ":" + p)
                    for p in ("before", "during", "after")
                )

            def checkpoint(data, output):
                data["results"] = summarize(data)

            probe = Mock()
            probe.sample.return_value = f["baseline"]
            with (
                patch.object(gate, "gui_lock", return_value=nullcontext()),
                patch.object(gate, "measured_side", side_effect=measured),
                patch.object(gate, "save", side_effect=checkpoint),
                patch("builtins.print"),
            ):
                data = gate.run(config, Path(root), probe)
            self.assertEqual(len(data["attempts"]), 310)
            self.assertEqual(data["results"]["cold_launch"]["status"], "FAIL")
            self.assertEqual(len(data["results"]["cold_launch"]["accepted_pairs"]), 30)

    def test_interrupt_preserves_completed_observations(self):
        f = fixture()
        with tempfile.TemporaryDirectory(dir="/tmp") as root:

            def measured(config, row, side, attempt, scratch, probe, call):
                if side == "electron":
                    raise KeyboardInterrupt("synthetic interruption")
                attempt[side] = copy.deepcopy(f["rows"][row][side])

            probe = Mock()
            probe.sample.return_value = f["baseline"]
            config = dict(
                context=f["context"], apps={}, reference="/unused", scenarios={}
            )
            with (
                patch.object(gate, "gui_lock", return_value=nullcontext()),
                patch.object(gate, "measured_side", side_effect=measured),
            ):
                data = gate.run(config, Path(root), probe)
            self.assertIn("native", data["attempts"][0])
            self.assertTrue((Path(root) / "report.json").exists())
            self.assertTrue(any("interruption" in s for s in data["blockers"]))

    def test_300_independent_pairs_alternate_order_and_checkpoint_each_attempt(self):
        f = fixture()
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            root = Path(root)
            config = dict(
                context=f["context"],
                apps={"native": sys.executable, "electron": sys.executable},
                max_attempts_per_row=30,
                reference="/unused",
                scenarios={},
                retry_settle_s=0,
            )
            calls = []

            def measured(config, row, side, attempt, scratch, probe, call):
                calls.append((row, attempt["index"], side))
                attempt[side] = copy.deepcopy(f["rows"][row][side])
                attempt["conditions"].extend(
                    dict(f["baseline"], phase=side + ":" + p)
                    for p in ("before", "during", "after")
                )

            def checkpoint(data, output):
                data["results"] = summarize(data)

            probe = Mock()
            probe.sample.return_value = f["baseline"]
            with (
                patch.object(gate, "gui_lock", return_value=nullcontext()),
                patch.object(gate, "measured_side", side_effect=measured),
                patch.object(gate, "save", side_effect=checkpoint) as saved,
                patch("builtins.print"),
            ):
                data = gate.run(config, root, probe)
            self.assertEqual(len(calls), 600)
            self.assertEqual(saved.call_count, 301)
            for row in ROWS:
                selected = [c for c in calls if c[0] == row]
                self.assertEqual(selected[0][2], "native")
                self.assertEqual(selected[2][2], "electron")
            self.assertTrue(
                all(r["status"] == "PASS" for r in data["results"].values())
            )

    def test_preflight_blocks_without_opening_apps(self):
        f = fixture()
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            probe = Mock()
            probe.sample.return_value = dict(f["baseline"], thermal="serious")
            with patch.object(gate, "measured_side") as measured:
                result = gate.run(
                    dict(context=f["context"], apps={}), Path(root), probe
                )
            measured.assert_not_called()
            self.assertEqual(len(result["results"]), 10)
            self.assertTrue((Path(root) / "report.md").exists())

    def test_measured_side_uses_private_writable_copy_and_samples_during(self):
        f = fixture()
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            root = Path(root)
            source = root / "source.json"
            source.write_text('{"public":true}')
            scratch = root / "native"
            scratch.mkdir()
            config = dict(
                context=f["context"],
                reference="/unused",
                apps={},
                scenarios={
                    "open": {"project": str(source), "sha256": gate.digest(source)}
                },
            )
            probe = Mock()
            probe.sample.return_value = f["baseline"]

            def call(argv, request, timeout):
                self.assertNotEqual(Path(request["project"]), source)
                Path(request["project"]).write_text("app save")
                self.assertEqual(request["scenario"]["sha256"], gate.digest(source))
                return f["rows"]["open"]["native"]

            attempt = dict(index=0, conditions=[])
            gate.measured_side(config, "open", "native", attempt, scratch, probe, call)
            self.assertEqual(source.read_text(), '{"public":true}')
            self.assertEqual(
                {s["phase"] for s in attempt["conditions"]},
                {"native:before", "native:during", "native:after"},
            )

    def test_replay_command_outputs_table_and_never_certifies(self):
        command = [
            sys.executable,
            str(HERE / "gate.py"),
            "p1",
            "--replay",
            str(HERE / "fixtures/recorded-p1.json"),
        ]
        result = subprocess.run(command, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn("REPLAY ONLY", result.stdout)
        self.assertIn("24.000 / 24.000 (n=30)", result.stdout)
        self.assertIn("Native own share", result.stdout)

    def test_adapter_protocol_and_timeout_are_bounded(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            request = dict(
                scratch=root,
                version=1,
                row="open",
                side="native",
                index=0,
                scenario={"sha256": "synthetic"},
            )
            script = (
                "import json,sys; r=json.load(sys.stdin); "
                'r.update(metrics={},evidence={},scenario_sha256=r["scenario"]["sha256"]); print(json.dumps(r))'
            )
            self.assertEqual(
                gate.invoke([sys.executable, "-c", script], request, timeout=2)[
                    "metrics"
                ],
                {},
            )
            for response in (
                "{}",
                '{"metrics":{},"evidence":{}}',
                '{"metrics":{"x":NaN},"evidence":{}}',
            ):
                with self.assertRaises(ValueError):
                    gate.invoke(
                        [sys.executable, "-c", "print(" + repr(response) + ")"],
                        request,
                        timeout=2,
                    )
            with self.assertRaises(subprocess.TimeoutExpired):
                gate.invoke(
                    [sys.executable, "-c", "import time; time.sleep(30)"],
                    request,
                    timeout=0.05,
                )
            cancelled = threading.Event()
            cancelled.set()
            groups = []
            with self.assertRaisesRegex(RuntimeError, "cancelled"):
                gate.invoke(
                    [sys.executable, "-c", "import time; time.sleep(30)"],
                    request,
                    timeout=30,
                    cancel=cancelled,
                    group_started=lambda pid: groups.append(("started", pid)),
                    group_finished=lambda pid: groups.append(("finished", pid)),
                )
            self.assertEqual([g[0] for g in groups], ["started", "finished"])
            self.assertEqual(groups[0][1], groups[1][1])


class ConfigurationTests(unittest.TestCase):
    def prepare(self, root):
        project = root / "public.json"
        project.write_text('{"fps":60,"dur":3,"layers":[]}')
        media = root / "public-video.mp4"
        media.write_bytes(b"synthetic test media, ffprobe is injected")
        return SimpleNamespace(
            shape=project,
            video=project,
            audio=project,
            video_media=media,
            native=Path(sys.executable),
            electron=Path(sys.executable),
            reference=root / "reference",
            hz=120,
            rig="xdr-120",
            certify_rig=False,
            certify_quality=False,
            quality_note="",
            allow_app=[],
            electron_arg=[],
        )

    def probe(self, argv, **kwargs):
        if argv[0] == "ffprobe":
            return SimpleNamespace(
                returncode=0,
                stdout=json.dumps(
                    {
                        "streams": [
                            {"width": 3840, "height": 2160, "codec_name": "hevc"}
                        ],
                        "frames": [
                            {"key_frame": 1, "best_effort_timestamp_time": "0"},
                            {"key_frame": 1, "best_effort_timestamp_time": "2"},
                        ],
                    }
                ),
            )
        return SimpleNamespace(returncode=0, stdout=gate.FROZEN_REFERENCE)

    def test_manifest_covers_all_rows_and_probes_actual_gop_before_runs(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            args = self.prepare(Path(root))
            config = configure.build(args)
            with patch.object(gate.subprocess, "run", side_effect=self.probe):
                actual = gate.validate(config)
            self.assertEqual(set(actual["scenarios"]), set(ROWS))
            self.assertTrue(
                actual["scenarios"]["scrub_random_seek"]["long_gop_4k_verified"]
            )
            self.assertFalse(actual["context"]["rig_certified"])

    def test_hash_mismatch_wrong_reference_and_bad_config_fail_before_apps(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            args = self.prepare(Path(root))
            for mutate in (
                "hash",
                "fps",
                "attempts",
                "timeout",
                "quality",
                "rig",
                "load",
            ):
                config = configure.build(args)
                if mutate == "hash":
                    config["scenarios"]["open"]["sha256"] = "wrong"
                if mutate == "fps":
                    config["scenarios"]["playback"]["fps"] = 30
                if mutate == "attempts":
                    config["max_attempts_per_row"] = 29
                if mutate == "timeout":
                    config["side_timeout_s"] = 900
                if mutate == "quality":
                    config["context"]["quality_certified"] = True
                if mutate == "rig":
                    config["context"]["rig"] = "base-60"
                if mutate == "load":
                    config["context"]["max_load_per_cpu"] = 12
                with (
                    patch.object(gate.subprocess, "run", side_effect=self.probe),
                    self.assertRaises(ValueError),
                ):
                    gate.validate(config)
            with patch.object(
                gate.subprocess,
                "run",
                return_value=SimpleNamespace(returncode=0, stdout="wrong"),
            ):
                with self.assertRaisesRegex(ValueError, "frozen"):
                    gate.validate(configure.build(args))

    def test_unsupported_codec_or_short_gop_cannot_certify_random_seek(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            args = self.prepare(Path(root))
            for change in ("codec", "size", "keyframes"):

                def probe(argv, **kwargs):
                    value = self.probe(argv, **kwargs)
                    if argv[0] == "ffprobe":
                        info = json.loads(value.stdout)
                        if change == "codec":
                            info["streams"][0]["codec_name"] = "prores"
                        if change == "size":
                            info["streams"][0]["width"] = 1920
                        if change == "keyframes":
                            info["frames"] = info["frames"][:1]
                        value.stdout = json.dumps(info)
                    return value

                with (
                    patch.object(gate.subprocess, "run", side_effect=probe),
                    self.assertRaisesRegex(ValueError, "long-GOP"),
                ):
                    gate.validate(configure.build(args))


class CollectorTests(unittest.TestCase):
    def request(self, root, row, side="native"):
        return dict(
            row=row,
            side=side,
            scratch=str(root),
            scenario={"fps": 60, "frames": 600},
            index=0,
            apps={"native": "/unused", "electron": "/unused"},
            reference="/unused",
            project=str(root / "project.json"),
        )

    def app(self, root):
        app = Mock(
            kind="native",
            pid=123,
            ready_ms=100,
            ready={"endpoint": "harness-ready"},
            trace_path=root / "absent.trace",
        )
        app.action.return_value = dict(
            own_share_ms=3, on_screen_ms=24, physical_present=True, cache_hit=True
        )
        app.request.return_value = {"playing": True, "frame": {"ticks": 0}}
        return app

    def test_rows_drive_existing_commands_and_close_app(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            for row in ROWS:
                app = self.app(root)
                observation = dict(
                    duration_s=60, cpu_percent=10, family_phys_footprint_mb=200
                )
                with patch.object(collect.time, "sleep"):
                    value = collect.collect(
                        self.request(root, row),
                        lambda *_: app,
                        Mock,
                        lambda *_: observation,
                    )
                app.close.assert_called_once()
                self.assertIn("metrics", value)
                if row == "cold_launch":
                    self.assertNotIn("latency_ms", value["metrics"])
                if row == "playback_cost":
                    self.assertEqual(value["metrics"]["cpu_percent"], 10)
                    self.assertNotIn("energy_j", value["metrics"])
                if row == "idle":
                    self.assertNotIn("extension_wakeups_s", value["metrics"])

    def test_stopped_playback_and_action_error_close_app(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            app = self.app(root)
            app.request.return_value = {"playing": False, "frame": {"ticks": 0}}
            with self.assertRaisesRegex(RuntimeError, "Playback stopped"):
                collect.collect(
                    self.request(root, "playback"),
                    lambda *_: app,
                    Mock,
                    lambda *_: {"duration_s": 60},
                )
            app.close.assert_called_once()
            app = self.app(root)
            app.action.side_effect = RuntimeError("forced action failure")
            with self.assertRaisesRegex(RuntimeError, "forced action failure"):
                collect.collect(
                    self.request(root, "open"), lambda *_: app, Mock, Mock()
                )
            app.close.assert_called_once()

    def test_random_seek_uses_identical_seeded_frames_for_both_apps(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            targets = []
            for side in ("native", "electron"):
                app = self.app(root)
                value = collect.collect(
                    self.request(root, "scrub_random_seek", side), lambda *_: app, Mock
                )
                targets.append(value["diagnostics"]["target_frame"])
                self.assertNotIn("preview_ms", value["metrics"])
            self.assertEqual(targets[0], targets[1])

    def test_raf_result_is_not_upgraded_to_presented_time(self):
        metrics, evidence = collect.latency(
            dict(own_share_ms=3, ms=15, on_screen_ms=None, physical_present=False)
        )
        self.assertEqual(metrics, {"own_share_ms": 3})
        self.assertIsNone(evidence["screen_endpoint"])


class ElectronTests(unittest.TestCase):
    @unittest.skipUnless(
        shutil.which("node"), "Node is required for the Electron protocol double"
    )
    def test_configured_electron_is_isolated_and_reports_two_distinct_endpoints(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as root:
            root = Path(root)
            ref = root / "reference"
            package = ref / "apps/desktop/node_modules/playwright"
            package.mkdir(parents=True)
            (ref / "apps/desktop/package.json").write_text(
                '{"name":"synthetic-fixture"}'
            )
            shutil.copyfile(HERE / "fixtures/playwright.cjs", package / "index.js")
            project = root / "document.json"
            project.write_text(
                '{"id":"synthetic","layers":[{"id":"shape","type":"shape"}]}'
            )
            profile, record = root / "profile", root / "launch.json"
            commands = [
                dict(op="open", path=str(project)),
                dict(op="seek", seconds=1 / 60),
                dict(op="seek", seconds=0),
                dict(op="edit", value=37),
                dict(op="invalid"),
                dict(op="quit"),
            ]
            env = dict(
                os.environ,
                PM_PERF_ELECTRON_PATH=str(root / "Powermove"),
                PM_PERF_ELECTRON_PROFILE=str(profile),
                PM_PERF_ELECTRON_ARGS="[]",
                PM_PERF_TEST_LAUNCH=str(record),
                ELECTRON_RUN_AS_NODE="1",
            )
            completed = subprocess.run(
                [shutil.which("node"), str(HERE / "electron.mjs"), str(ref)],
                input="\n".join(json.dumps(c) for c in commands) + "\n",
                capture_output=True,
                text=True,
                env=env,
                timeout=10,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            rows = [
                json.loads(line[8:])
                for line in completed.stdout.splitlines()
                if line.startswith("PM_PERF ")
            ]
            self.assertTrue(rows[0]["ready"])
            self.assertEqual(len(rows), 6)
            for value in rows[1:5]:
                self.assertGreaterEqual(value["ms"], value["own_share_ms"])
                self.assertIsNone(value["on_screen_ms"])
                self.assertFalse(value["physical_present"])
            self.assertEqual(rows[5]["error"], "Unknown operation")
            launch = json.loads(record.read_text())
            self.assertEqual(launch["profile"], str(profile))
            self.assertEqual(launch["runtimeUpdates"], "1")
            self.assertEqual(launch["background"], "0")
            self.assertIsNone(launch["runAsNode"])
            self.assertTrue((profile / "onboarding-v1.json").exists())
            marker = profile / "preserve-existing-data"
            marker.write_text("synthetic retained value")
            repeated = subprocess.run(
                [shutil.which("node"), str(HERE / "electron.mjs"), str(ref)],
                input='{"op":"quit"}\n',
                capture_output=True,
                text=True,
                env=env,
                timeout=10,
            )
            self.assertEqual(repeated.returncode, 1)
            self.assertIn("EEXIST", repeated.stdout)
            self.assertEqual(marker.read_text(), "synthetic retained value")


if __name__ == "__main__":
    unittest.main()
