use pm_fx_defs::lut::parse;
#[test]
fn main_cube_layout_domains_and_rejections() {
    let one=parse("\u{feff}TITLE \"A # title\"\nLUT_1D_SIZE 2\nDOMAIN_MIN -1 -2 -3\nDOMAIN_MAX 1 2 3\n0 1 2\n3 4 5\n").unwrap();
    assert_eq!((one.size, one.width, one.height), (2, 2, 1));
    assert_eq!(one.rgba, [0., 1., 2., 1., 3., 4., 5., 1.]);
    let cube = format!(
        "LUT_3D_SIZE 2\n{}",
        (0..8)
            .map(|i| format!("{} {} {}\n", i & 1, (i >> 1) & 1, (i >> 2) & 1))
            .collect::<String>()
    );
    let three = parse(&cube).unwrap();
    assert_eq!((three.width, three.height), (4, 2));
    for bad in [
        "LUT_1D_SIZE 2\n0 0 0",
        "LUT_3D_SIZE 65",
        "LUT_1D_SIZE 4097",
        "LUT_3D_SIZE 2\nLUT_1D_SIZE 2",
        "LUT_1D_SIZE 2\n0 0 0\nNaN 0 0",
        "LUT_1D_SIZE 2\n0 0 0\n65505 0 0",
        "DOMAIN_MIN 1 1 1\nDOMAIN_MAX 1 1 1\nLUT_1D_SIZE 2\n0 0 0\n1 1 1",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}
