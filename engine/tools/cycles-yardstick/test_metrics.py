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

    def test_film_quadrature_matches_generator_and_preserves_neutral_spectra(self):
        from pathlib import Path
        from generate_film_response import text, response
        shader = Path(__file__).resolve().parents[2]/'crates/pm-studio/src/shaders/trace.wgsl'
        self.assertIn(text(),shader.read_text())
        np.testing.assert_allclose(response().sum(axis=0),[1,1,1],atol=1e-12)

    def test_reference_conductor_preserves_both_reflectance_constraints(self):
        from mapping import conductor_ior
        for f0 in [.04,.3,.6,.9]:
            n,k = conductor_ior(f0)
            eta = complex(n,k)
            normal = abs((1-eta)/(1+eta))**2
            self.assertAlmostEqual(normal,f0,places=8)
            c = 1/7
            q = (eta*eta-(1-c*c))**.5
            value = (abs((c-q)/(c+q))**2 + abs((eta*eta*c-q)/(eta*eta*c+q))**2)/2
            self.assertAlmostEqual(value,f0+(1-f0)*(6/7)**5,delta=5e-4)

    def test_reference_anisotropy_preserves_both_ggx_widths(self):
        from mapping import anisotropic_widths
        for k in [.1,.5,.8]:
            roughness, anisotropic = anisotropic_widths(.3,k)
            aspect = (1-.9*anisotropic)**.5
            self.assertAlmostEqual(roughness**2/aspect,.3**2*(1+k))
            self.assertAlmostEqual(roughness**2*aspect,.3**2*(1-k))

    def test_replay_script_contains_pure_mapping_helpers(self):
        from run import exporter_script
        self.assertIn('def conductor_ior(',exporter_script())
        compile(exporter_script(),'render_cycles.py','exec')

    def test_sheen_energy_matches_quadrature_and_is_bounded(self):
        from pathlib import Path
        from generate_sheen_lut import text,tables
        shader = Path(__file__).resolve().parents[2]/'crates/pm-studio/src/shaders/trace.wgsl'
        self.assertIn(text(),shader.read_text())
        values,average = tables()
        normalized = values/np.maximum(1,values.max(axis=1))[:,None]
        self.assertTrue(np.all((normalized>=0)&(normalized<=1)))
        self.assertTrue(np.all((average>=0)&(average<1)))

    def test_reused_reference_can_be_reused_again_and_rejects_stale_mapping(self):
        import json
        import tempfile
        from pathlib import Path
        from run import exporter_script,reuse_reference
        with tempfile.TemporaryDirectory() as directory:
            source,first,second = [Path(directory)/name for name in ['source','first','second']]
            for folder in [source,first,second]:
                folder.mkdir()
                (folder/'scene.json').write_text('{}')
                (folder/'manifest.json').write_text(json.dumps({'size':[2,1]}))
            (source/'render_cycles.py').write_text(exporter_script())
            for name in ['cycles.exr','cycles.npy','cycles-meta.json']:
                (source/name).write_bytes(b'test-only reference fixture')
            reuse_reference(source,first)
            reuse_reference(first,second)
            self.assertEqual((second/'render_cycles.py').read_text(),exporter_script())
            self.assertEqual((second/'cycles.npy').read_bytes(),(source/'cycles.npy').read_bytes())
            (first/'render_cycles.py').write_text('# stale material mapping')
            with self.assertRaisesRegex(ValueError,'exporter'):
                reuse_reference(first,second)
            (first/'render_cycles.py').unlink()
            with self.assertRaisesRegex(ValueError,'replay proof'):
                reuse_reference(first,second)

    def test_absent_blender_skips_without_building(self):
        import run
        with patch('sys.argv',['run.py']), patch.object(run,'blender_path',return_value=None), patch('subprocess.run') as child:
            self.assertEqual(run.main(),0)
            child.assert_not_called()


if __name__ == '__main__':
    unittest.main()
