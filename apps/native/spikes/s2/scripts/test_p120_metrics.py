"""Run with: python3 -m unittest discover -s spikes/s2/scripts -p 'test_p120_metrics.py'."""
import tempfile
import unittest
from pathlib import Path
from p120_metrics import metrics


class EvidenceTests(unittest.TestCase):
    def parse(self, *, omit=None, zero=None, duplicate=None, gap=None, duration='3ms'):
        lines = []
        # Includes 30 warmups and >60 s of measured 120-Hz frames.
        for i in range(7240):
            lines.extend([f'frame duration: {duration}', 'PM_GPUI_MAIN_CPU_MS 2.5',
                          'GPUI drawable: 10µs'])
            t = 100 + i / 120 + (1 / 120 if gap is not None and i >= gap else 0)
            if i == omit:
                continue
            if i == zero:
                t = 0
            if i == duplicate:
                t = 100 + (i - 1) / 120
            lines.append(f'PM_GPUI_PRESENT {t:.9f}')
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'fixture.log'
            path.write_text('\n'.join(lines))
            return metrics(path)

    def test_complete_120hz_evidence(self):
        row = self.parse()
        self.assertTrue(row['pass'])
        self.assertEqual(row['missed_120hz_intervals'], 0)
        self.assertEqual(row['main_thread_cpu_p95_ms'], 2.5)
        self.assertEqual(row['drawable_wait_p95_ms'], .01)

    def test_zero_missing_and_duplicate_timestamps_invalidate(self):
        for kwargs in [dict(zero=200), dict(omit=200), dict(duplicate=200)]:
            with self.subTest(kwargs=kwargs):
                row = self.parse(**kwargs)
                self.assertFalse(row['presentation_evidence_valid'])
                self.assertFalse(row['pass'])

    def test_real_gap_is_counted_even_with_enough_presentations(self):
        row = self.parse(gap=200)
        self.assertTrue(row['presentation_evidence_valid'])
        self.assertEqual(row['missed_120hz_intervals'], 1)
        self.assertFalse(row['pass'])

    def test_headroom_uses_full_main_thread_wall_duration(self):
        row = self.parse(duration='5ms')
        self.assertEqual(row['main_thread_cpu_p95_ms'], 2.5)
        self.assertFalse(row['pass'], 'CPU running time must not hide blocking waits')


if __name__ == '__main__':
    unittest.main()
