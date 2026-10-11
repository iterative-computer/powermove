//! WebGL2 enum values used by the replayer.
#![allow(dead_code)]

pub const DEPTH_BUFFER_BIT: u32 = 0x0100;
pub const STENCIL_BUFFER_BIT: u32 = 0x0400;
pub const COLOR_BUFFER_BIT: u32 = 0x4000;

pub const POINTS: u32 = 0;
pub const LINES: u32 = 1;
pub const LINE_LOOP: u32 = 2;
pub const LINE_STRIP: u32 = 3;
pub const TRIANGLES: u32 = 4;
pub const TRIANGLE_STRIP: u32 = 5;
pub const TRIANGLE_FAN: u32 = 6;

pub const ZERO: u32 = 0;
pub const ONE: u32 = 1;
pub const SRC_COLOR: u32 = 0x0300;
pub const ONE_MINUS_SRC_COLOR: u32 = 0x0301;
pub const SRC_ALPHA: u32 = 0x0302;
pub const ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
pub const DST_ALPHA: u32 = 0x0304;
pub const ONE_MINUS_DST_ALPHA: u32 = 0x0305;
pub const DST_COLOR: u32 = 0x0306;
pub const ONE_MINUS_DST_COLOR: u32 = 0x0307;
pub const SRC_ALPHA_SATURATE: u32 = 0x0308;
pub const CONSTANT_COLOR: u32 = 0x8001;
pub const ONE_MINUS_CONSTANT_COLOR: u32 = 0x8002;
pub const CONSTANT_ALPHA: u32 = 0x8003;
pub const ONE_MINUS_CONSTANT_ALPHA: u32 = 0x8004;
pub const FUNC_ADD: u32 = 0x8006;
pub const MIN: u32 = 0x8007;
pub const MAX: u32 = 0x8008;
pub const FUNC_SUBTRACT: u32 = 0x800A;
pub const FUNC_REVERSE_SUBTRACT: u32 = 0x800B;

pub const ARRAY_BUFFER: u32 = 0x8892;
pub const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
pub const UNIFORM_BUFFER: u32 = 0x8A11;
pub const COPY_READ_BUFFER: u32 = 0x8F36;
pub const COPY_WRITE_BUFFER: u32 = 0x8F37;
pub const PIXEL_UNPACK_BUFFER: u32 = 0x88EC;

pub const FRONT: u32 = 0x0404;
pub const BACK: u32 = 0x0405;
pub const FRONT_AND_BACK: u32 = 0x0408;
pub const CW: u32 = 0x0900;
pub const CCW: u32 = 0x0901;

pub const CULL_FACE: u32 = 0x0B44;
pub const BLEND: u32 = 0x0BE2;
pub const DITHER: u32 = 0x0BD0;
pub const STENCIL_TEST: u32 = 0x0B90;
pub const DEPTH_TEST: u32 = 0x0B71;
pub const SCISSOR_TEST: u32 = 0x0C11;
pub const POLYGON_OFFSET_FILL: u32 = 0x8037;
pub const SAMPLE_ALPHA_TO_COVERAGE: u32 = 0x809E;
pub const SAMPLE_COVERAGE: u32 = 0x80A0;
pub const RASTERIZER_DISCARD: u32 = 0x8C89;

pub const NEVER: u32 = 0x0200;
pub const LESS: u32 = 0x0201;
pub const EQUAL: u32 = 0x0202;
pub const LEQUAL: u32 = 0x0203;
pub const GREATER: u32 = 0x0204;
pub const NOTEQUAL: u32 = 0x0205;
pub const GEQUAL: u32 = 0x0206;
pub const ALWAYS: u32 = 0x0207;

pub const BYTE: u32 = 0x1400;
pub const UNSIGNED_BYTE: u32 = 0x1401;
pub const SHORT: u32 = 0x1402;
pub const UNSIGNED_SHORT: u32 = 0x1403;
pub const INT: u32 = 0x1404;
pub const UNSIGNED_INT: u32 = 0x1405;
pub const FLOAT: u32 = 0x1406;
pub const HALF_FLOAT: u32 = 0x140B;
pub const UNSIGNED_INT_24_8: u32 = 0x84FA;
pub const UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
pub const UNSIGNED_SHORT_5_5_5_1: u32 = 0x8034;
pub const UNSIGNED_SHORT_5_6_5: u32 = 0x8363;

pub const DEPTH_COMPONENT: u32 = 0x1902;
pub const ALPHA: u32 = 0x1906;
pub const RGB: u32 = 0x1907;
pub const RGBA: u32 = 0x1908;
pub const LUMINANCE: u32 = 0x1909;
pub const LUMINANCE_ALPHA: u32 = 0x190A;
pub const RED: u32 = 0x1903;
pub const RG: u32 = 0x8227;
pub const DEPTH_STENCIL: u32 = 0x84F9;
pub const RED_INTEGER: u32 = 0x8D94;
pub const RG_INTEGER: u32 = 0x8228;
pub const RGB_INTEGER: u32 = 0x8D98;
pub const RGBA_INTEGER: u32 = 0x8D99;

pub const R8: u32 = 0x8229;
pub const RG8: u32 = 0x822B;
pub const R16F: u32 = 0x822D;
pub const R32F: u32 = 0x822E;
pub const RG16F: u32 = 0x822F;
pub const RG32F: u32 = 0x8230;
pub const RGB8: u32 = 0x8051;
pub const RGBA8: u32 = 0x8058;
pub const SRGB8: u32 = 0x8C41;
pub const SRGB8_ALPHA8: u32 = 0x8C43;
pub const RGBA32F: u32 = 0x8814;
pub const RGB32F: u32 = 0x8815;
pub const RGBA16F: u32 = 0x881A;
pub const RGB16F: u32 = 0x881B;
pub const DEPTH_COMPONENT16: u32 = 0x81A5;
pub const DEPTH_COMPONENT24: u32 = 0x81A6;
pub const DEPTH_COMPONENT32F: u32 = 0x8CAC;
pub const DEPTH24_STENCIL8: u32 = 0x88F0;
pub const DEPTH32F_STENCIL8: u32 = 0x8CAD;

pub const TEXTURE_2D: u32 = 0x0DE1;
pub const TEXTURE_CUBE_MAP: u32 = 0x8513;
pub const TEXTURE_CUBE_MAP_POSITIVE_X: u32 = 0x8515;
pub const TEXTURE_3D: u32 = 0x806F;
pub const TEXTURE_2D_ARRAY: u32 = 0x8C1A;
pub const TEXTURE0: u32 = 0x84C0;

pub const TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const TEXTURE_WRAP_S: u32 = 0x2802;
pub const TEXTURE_WRAP_T: u32 = 0x2803;
pub const TEXTURE_WRAP_R: u32 = 0x8072;
pub const TEXTURE_COMPARE_MODE: u32 = 0x884C;
pub const TEXTURE_COMPARE_FUNC: u32 = 0x884D;
pub const COMPARE_REF_TO_TEXTURE: u32 = 0x884E;
pub const TEXTURE_BASE_LEVEL: u32 = 0x813C;
pub const TEXTURE_MAX_LEVEL: u32 = 0x813D;
pub const TEXTURE_MAX_ANISOTROPY_EXT: u32 = 0x84FE;

pub const NEAREST: u32 = 0x2600;
pub const LINEAR: u32 = 0x2601;
pub const NEAREST_MIPMAP_NEAREST: u32 = 0x2700;
pub const LINEAR_MIPMAP_NEAREST: u32 = 0x2701;
pub const NEAREST_MIPMAP_LINEAR: u32 = 0x2702;
pub const LINEAR_MIPMAP_LINEAR: u32 = 0x2703;
pub const REPEAT: u32 = 0x2901;
pub const CLAMP_TO_EDGE: u32 = 0x812F;
pub const MIRRORED_REPEAT: u32 = 0x8370;

pub const FRAMEBUFFER: u32 = 0x8D40;
pub const READ_FRAMEBUFFER: u32 = 0x8CA8;
pub const DRAW_FRAMEBUFFER: u32 = 0x8CA9;
pub const RENDERBUFFER: u32 = 0x8D41;
pub const COLOR_ATTACHMENT0: u32 = 0x8CE0;
pub const DEPTH_ATTACHMENT: u32 = 0x8D00;
pub const STENCIL_ATTACHMENT: u32 = 0x8D20;
pub const DEPTH_STENCIL_ATTACHMENT: u32 = 0x821A;
pub const NONE: u32 = 0;

pub const UNPACK_ALIGNMENT: u32 = 0x0CF5;
pub const PACK_ALIGNMENT: u32 = 0x0D05;
pub const UNPACK_ROW_LENGTH: u32 = 0x0CF2;
pub const UNPACK_SKIP_ROWS: u32 = 0x0CF3;
pub const UNPACK_SKIP_PIXELS: u32 = 0x0CF4;
pub const UNPACK_IMAGE_HEIGHT: u32 = 0x806E;
pub const UNPACK_SKIP_IMAGES: u32 = 0x806D;
pub const UNPACK_FLIP_Y_WEBGL: u32 = 0x9240;
pub const UNPACK_PREMULTIPLY_ALPHA_WEBGL: u32 = 0x9241;
pub const UNPACK_COLORSPACE_CONVERSION_WEBGL: u32 = 0x9243;

pub const VERTEX_SHADER: u32 = 0x8B31;
pub const FRAGMENT_SHADER: u32 = 0x8B30;
