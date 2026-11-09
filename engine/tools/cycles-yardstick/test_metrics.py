"""Cheap deterministic tests of the yardstick; no Blender or GPU needed."""
import unittest
from unittest.mock import patch
import numpy as np
from metrics import delta_e2000, exposure_match, lab, neutral, score, ssim_map


class MetricsTests(unittest.TestCase):
    def test_published_ciede2000_pairs(self):
        # Sharma, Wu, Dalal supplementary test data, pairs 1-4 and neutral.
        pairs = [([50,2.6772,-79.7751],[50,0,-82.7485],2.0425),
                 ([50,3.1571,-77.2803],[50,0,-82.7485],2.8615),
                 ([50,2.8361,-74.0200],[50,0,-82.7485],3.4412),
                 ([50,-1.3802,-84.2814],[50,0,-82.7485],1.0000),
                 ([50,0,0],[50,-1,2],2.3669)]
        for a,b,expected in pairs:
            self.assertAlmostEqual(float(delta_e2000(a,b)),expected,places=4)
            self.assertAlmostEqual(float(delta_e2000(b,a)),expected,places=4)

    def test_exposure_fit_is_one_scalar_and_reports_raw_error(self):
        a = np.full((16,16,3), .2)
        b = a*2
        mask = np.ones((16,16),bool)
        gain = exposure_match(a,b,mask)
        self.assertAlmostEqual(gain,2)
        result = score(a,b,{'whole':mask},gain)['whole']
        self.assertLess(result['delta_e_mean'],1e-10)
        self.assertAlmostEqual(result['raw_luminance_ratio'],.5)

    def test_identity_black_and_hdr_are_finite(self):
        a = np.random.default_rng(42).uniform(0,8,(16,16,3))
        self.assertTrue(np.allclose(ssim_map(a,a),1))
        self.assertTrue(np.isfinite(neutral(a)).all())
        self.assertTrue(np.isfinite(neutral(np.zeros_like(a))).all())
        self.assertLess(np.max(delta_e2000(lab(neutral(a)),lab(neutral(a)))),1e-10)

    def test_display_scores_do_not_apply_a_second_view_transform(self):
        a, b = np.ones((16,16,3)), np.zeros((16,16,3))
        result = score(a,b,{'whole':np.ones((16,16),bool)},tone_map=False)['whole']
        self.assertAlmostEqual(result['delta_e_mean'],100,places=3)

    def test_energy_constants_match_quadrature(self):
        from pathlib import Path
        from generate_energy_lut import text
        shader = Path(__file__).resolve().parents[2]/'crates/pm-studio/src/shaders/trace.wgsl'
        self.assertIn(text(),shader.read_text())

    def test_absent_blender_skips_without_building(self):
        import run
        with patch('sys.argv',['run.py']), patch.object(run,'blender_path',return_value=None), patch('subprocess.run') as child:
            self.assertEqual(run.main(),0)
            child.assert_not_called()


if __name__ == '__main__':
    unittest.main()
