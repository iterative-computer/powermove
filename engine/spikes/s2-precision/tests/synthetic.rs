use s2_precision::gpu::{Gpu, PipeParams, Precision};
use s2_precision::pixbuf::{self, PixelBuffer};

#[test]
fn synthetic_y416_f32_round_trip_is_exact() {
    let gpu = Gpu::new().unwrap();
    let (w, h) = (256usize, 64usize);
    let src = PixelBuffer::new(w, h, pixbuf::Y416).unwrap();
    let mut data = vec![0u16; w * h * 4];
    let mut s: u32 = 1;
    for px in data.chunks_mut(4) {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        px[0] = 65535;
        px[1] = (16 + (s >> 8) % (4079 - 16)) as u16 * 16;
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        px[2] = (16 + (s >> 8) % (4079 - 16)) as u16 * 16;
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        px[3] = (16 + (s >> 8) % (4079 - 16)) as u16 * 16;
    }
    src.write_u16(None, &data).unwrap();
    let out = gpu.engine_copy(&src, &PipeParams::standard(Precision::F32)).unwrap();
    let (got, _, _) = out.read_u16(None).unwrap();
    let bad = data.iter().zip(&got).filter(|(a, b)| a != b).count();
    let maxd = data.iter().zip(&got).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert_eq!(bad, 0, "mismatches {bad}, max {maxd}");
}
