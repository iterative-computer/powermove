use pm_color::{lut::CubeLut, scopes::*};
fn cube(rows: &[[f64; 3]]) -> String {
    let mut s = String::from("LUT_3D_SIZE 2\n");
    for row in rows {
        s.push_str(&format!("{} {} {}\n", row[0], row[1], row[2]));
    }
    s
}
fn identity() -> String {
    cube(
        &(0..8)
            .map(|i| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64])
            .collect::<Vec<_>>(),
    )
}
fn near(a: [f64; 3], b: [f64; 3]) {
    for c in 0..3 {
        assert!((a[c] - b[c]).abs() < 1e-12, "{a:?} != {b:?}");
    }
}
#[test]
fn cube_identity_red_fastest_and_grid() {
    let lut = CubeLut::parse(&identity()).unwrap();
    for r in 0..=10 {
        for g in 0..=10 {
            for b in 0..=10 {
                let p = [r as f64 / 10., g as f64 / 10., b as f64 / 10.];
                near(lut.apply(p).unwrap(), p);
            }
        }
    }
}
#[test]
fn all_six_tetrahedra_use_four_corners() {
    // Corner values [0,1,4,9,16,25,36,49]; expected weighted sums
    // derive directly from the four vertices of each geometric tetrahedron.
    let lut = CubeLut::parse(&cube(
        &(0..8).map(|n| [(n * n) as f64; 3]).collect::<Vec<_>>(),
    ))
    .unwrap();
    for (p, vertices) in [
        ([0.75, 0.5, 0.25], [0, 1, 3, 7]),
        ([0.75, 0.25, 0.5], [0, 1, 5, 7]),
        ([0.5, 0.75, 0.25], [0, 2, 3, 7]),
        ([0.25, 0.75, 0.5], [0, 2, 6, 7]),
        ([0.5, 0.25, 0.75], [0, 4, 5, 7]),
        ([0.25, 0.5, 0.75], [0, 4, 6, 7]),
    ] {
        let expected = vertices.iter().map(|v| (v * v) as f64 * 0.25).sum();
        near(lut.apply(p).unwrap(), [expected; 3]);
    }
}
#[test]
fn tetrahedral_differs_from_trilinear() {
    let mut rows = [[0.; 3]; 8];
    rows[7] = [1.; 3];
    let lut = CubeLut::parse(&cube(&rows)).unwrap();
    assert_eq!(lut.apply([0.5; 3]).unwrap(), [0.5; 3]); // trilinear would be .125
}
#[test]
fn ties_faces_and_endpoints() {
    let lut = CubeLut::parse(&identity()).unwrap();
    for p in [
        [0.; 3],
        [1.; 3],
        [0.5; 3],
        [0., 1., 0.],
        [1., 0., 1.],
        [0.3, 0.3, 1.],
        [1., 0.4, 0.4],
    ] {
        near(lut.apply(p).unwrap(), p);
    }
}
#[test]
fn one_d_channel_independent_and_extended_output() {
    let lut =
        CubeLut::parse("TITLE \"curve # one\" # comment\nLUT_1D_SIZE 3\n-1 0 0\n0 2 4\n3 4 8\n")
            .unwrap();
    assert_eq!(lut.title.as_deref(), Some("curve # one"));
    assert_eq!(lut.apply([0.25, 0.5, 0.75]).unwrap(), [-0.5, 2., 6.]);
    assert_eq!(lut.apply([-10., 2., 0.]).unwrap(), [-1., 4., 0.]);
}
#[test]
fn nonuniform_domain_per_channel() {
    let s = format!("DOMAIN_MIN -1 0 10\nDOMAIN_MAX 1 4 20\n{}", identity());
    let lut = CubeLut::parse(&s).unwrap();
    near(lut.apply([0., 1., 17.5]).unwrap(), [0.5, 0.25, 0.75]);
    assert_eq!(lut.apply([-2., 8., 25.]).unwrap(), [0., 1., 1.]);
}
#[test]
fn combined_shaper_then_cube_and_stage_ranges() {
    let s = format!(
        "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE -1 1\nLUT_3D_INPUT_RANGE 0 2\nLUT_3D_SIZE 2\n0 0 0\n2 2 2\n{}",
        identity().lines().skip(1).collect::<Vec<_>>().join("\n")
    );
    let lut = CubeLut::parse(&s).unwrap();
    near(lut.apply([0., -1., 1.]).unwrap(), [0.5, 0., 1.]);
}
#[test]
fn three_d_input_range() {
    let lut = CubeLut::parse(&format!("LUT_3D_INPUT_RANGE -2 2\n{}", identity())).unwrap();
    near(lut.apply([0., 1., -1.]).unwrap(), [0.5, 0.75, 0.25]);
}
#[test]
fn malformed_cube_inputs_are_rejected() {
    for s in [
        "",
        "0 0 0",
        "LUT_1D_SIZE 1\n",
        "LUT_3D_SIZE 130\n",
        "LUT_1D_SIZE 65537\n",
        "LUT_1D_SIZE -2\n",
        "LUT_1D_SIZE 2\n0 0 0\n",
        "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n2 2 2",
        "LUT_1D_SIZE 2\nLUT_1D_SIZE 2",
        "DOMAIN_MIN 1 1 1\nDOMAIN_MAX 0 0 0\nLUT_1D_SIZE 2\n0 0 0\n1 1 1",
        "LUT_1D_SIZE 2\nNaN 0 0\n1 1 1",
        "LUT_1D_SIZE 2\n0 0 0\n1 inf 1",
        "TITLE unquoted\n",
        "LUT_1D_SIZE 2\n0 0 0\nTITLE \"late\"\n1 1 1",
        "UNKNOWN 1 2",
        "LUT_1D_INPUT_RANGE 0 0\n",
        "LUT_3D_INPUT_RANGE 0 1\nLUT_1D_SIZE 2\n0 0 0\n1 1 1",
        "DOMAIN_MIN 0 0 0\nLUT_1D_INPUT_RANGE 0 1\nLUT_1D_SIZE 2\n0 0 0\n1 1 1",
    ] {
        assert!(CubeLut::parse(s).is_err(), "accepted {s}");
    }
}
/// An infinite input is outside the domain like any other and reads as the
/// nearer end; one that is not a number reads as the domain's low end.
#[test]
fn a_nonfinite_input_has_a_stated_lookup() {
    let lut = CubeLut::parse(&identity()).unwrap();
    let low = lut.apply([0., 0., 0.]).unwrap();
    let high = lut.apply([1., 0., 0.]).unwrap();
    assert_eq!(lut.apply([f64::NAN, 0., 0.]).unwrap(), low);
    assert_eq!(lut.apply([f64::NEG_INFINITY, 0., 0.]).unwrap(), low);
    assert_eq!(lut.apply([f64::INFINITY, 0., 0.]).unwrap(), high);
}
fn cfg() -> ScopeConfig {
    ScopeConfig {
        columns: 2,
        levels: 5,
        vector_bins: 5,
        ..ScopeConfig::default()
    }
}
#[test]
fn scopes_black_white_density_and_positions() {
    let s = compute(&[[0.; 3], [1.; 3]], 2, 1, cfg()).unwrap();
    assert_eq!(s.pixels, 2);
    assert_eq!(s.luma_histogram.bins, vec![1, 0, 0, 0, 1]);
    assert_eq!(s.waveform[0], 1);
    assert_eq!(s.waveform[9], 1);
    for p in &s.parade {
        assert_eq!(p, &s.waveform);
    }
    assert_eq!(s.vectorscope[12], 2);
}
#[test]
fn scopes_colour_bars_have_correct_luma_and_chroma() {
    let s = compute(
        &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        3,
        1,
        ScopeConfig {
            columns: 3,
            ..cfg()
        },
    )
    .unwrap();
    assert_eq!(s.waveform[3], 1);
    assert_eq!(s.waveform[10], 1);
    assert_eq!(s.waveform[2], 1);
    // Red Cb=-.11457,Cr=.5 => (2,4); blue Cb=.5,Cr=-.04585 => (4,2).
    assert_eq!(s.vectorscope[22], 1);
    assert_eq!(s.vectorscope[14], 1);
    assert_eq!(s.histogram[0].bins, vec![2, 0, 0, 0, 1]);
}
#[test]
fn scopes_preserve_extended_range_diagnostics() {
    let s = compute(&[[-0.5; 3], [2.; 3]], 1, 2, cfg()).unwrap();
    assert_eq!(s.luma_histogram.below, 1);
    assert_eq!(s.luma_histogram.above, 1);
    assert_eq!(s.histogram[2].below, 1);
    assert_eq!(s.histogram[2].above, 1);
    assert_eq!(s.vectorscope.iter().sum::<u64>(), 2);
}
#[test]
fn scopes_explicit_matrix_and_analysis_range() {
    let p = [[1., 0., 0.]];
    let a = compute(
        &p,
        1,
        1,
        ScopeConfig {
            levels: 101,
            columns: 1,
            ..cfg()
        },
    )
    .unwrap();
    let b = compute(
        &p,
        1,
        1,
        ScopeConfig {
            matrix: Matrix::Bt2020,
            levels: 101,
            columns: 1,
            ..cfg()
        },
    )
    .unwrap();
    assert_eq!(a.luma_histogram.bins[21], 1);
    assert_eq!(b.luma_histogram.bins[26], 1);
    let s = compute(
        &[[1.; 3]],
        1,
        1,
        ScopeConfig {
            min: 0.,
            max: 2.,
            ..cfg()
        },
    )
    .unwrap();
    assert_eq!(s.luma_histogram.bins[2], 1);
}
#[test]
fn scopes_conserve_every_sample() {
    let p: Vec<_> = (0..35).map(|i| [i as f64 / 30., 0.25, -0.2]).collect();
    let s = compute(&p, 7, 5, cfg()).unwrap();
    for d in &s.histogram {
        assert_eq!(d.bins.iter().sum::<u64>(), 35);
    }
    for d in &s.parade {
        assert_eq!(d.iter().sum::<u64>(), 35);
    }
    assert_eq!(s.waveform.iter().sum::<u64>(), 35);
    assert_eq!(s.vectorscope.iter().sum::<u64>(), 35);
}
#[test]
fn scopes_reject_bad_frames_and_unbounded_allocations() {
    assert_eq!(compute(&[], 0, 0, cfg()), Err(ScopeError::Dimensions));
    assert!(compute(&[[0.; 3]], 2, 1, cfg()).is_err());
    for conf in [
        ScopeConfig { levels: 1, ..cfg() },
        ScopeConfig {
            columns: 0,
            ..cfg()
        },
        ScopeConfig {
            vector_bins: usize::MAX,
            ..cfg()
        },
        ScopeConfig { max: 0., ..cfg() },
        ScopeConfig {
            min: f64::NAN,
            ..cfg()
        },
    ] {
        assert_eq!(
            compute(&[[0.; 3]], 1, 1, conf),
            Err(ScopeError::Configuration)
        );
    }
    assert_eq!(
        compute(&[[0., f64::NAN, 0.]], 1, 1, cfg()),
        Err(ScopeError::NonfinitePixel(0))
    );
}
