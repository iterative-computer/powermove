import unittest
from unittest.mock import patch
import resources

class ResourceTests(unittest.TestCase):
    def test_decimal_container_bytes_and_cached_measurement(self):
        with patch('resources.sys.platform', 'darwin'), patch.object(resources, '_sample', (0, 0)), patch('resources.time.monotonic', return_value=10), \
             patch('resources.subprocess.check_output', return_value='   Container Free Space: 5.0 GB (5000000000 Bytes) (exactly 9765625 512-Byte-Units)') as read:
            self.assertTrue(resources.disk_ready())
            self.assertEqual(resources.container_free_bytes(), 5_000_000_000)
            read.assert_called_once_with(['diskutil', 'info', '/'], text=True)
    def test_below_floor_and_missing_container_data_fail_closed(self):
        with patch('resources.sys.platform', 'darwin'), patch.object(resources, '_sample', (0, 0)), patch('resources.time.monotonic', return_value=10), \
             patch('resources.subprocess.check_output', return_value='Container Free Space: 5.0 GB (4999999999 Bytes)'):
            self.assertFalse(resources.disk_ready())
        with patch('resources.sys.platform', 'darwin'), patch.object(resources, '_sample', (0, 0)), patch('resources.time.monotonic', return_value=10), \
             patch('resources.subprocess.check_output', return_value='Volume Free Space: 20 GB'):
            with self.assertRaises(RuntimeError):resources.disk_ready()

    def test_non_macos_measures_the_build_filesystem_without_diskutil(self):
        from types import SimpleNamespace
        with patch('resources.sys.platform', 'linux'), patch.object(resources, '_sample', (0, 0)), \
             patch('resources.time.monotonic', return_value=10), \
             patch('resources.shutil.disk_usage', return_value=SimpleNamespace(free=5_000_000_000)), \
             patch('resources.subprocess.check_output') as diskutil:
            self.assertTrue(resources.disk_ready())
        diskutil.assert_not_called()
