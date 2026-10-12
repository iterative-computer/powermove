//! Character predicates ported from Blink (`platform/text/character.h`,
//! `character.cc`, `character_emoji.cc`, `character_property_data.h`) as of
//! Chromium 152. Unicode properties come from ICU4X, which uses the same
//! script codes as ICU4C.

use icu_properties::props::{
    BidiClass, BidiMirroringGlyph, BidiPairedBracketType, DefaultIgnorableCodePoint,
    EastAsianWidth, Emoji, EmojiComponent, EmojiModifierBase, EmojiPresentation,
    ExtendedPictographic, GeneralCategory, GeneralCategoryGroup, Ideographic, Script,
};
use icu_properties::{CodePointMapData, CodePointSetData, script::ScriptWithExtensions};

pub const SPACE: u32 = 0x20;
pub const TAB: u32 = 0x09;
pub const LINE_FEED: u32 = 0x0A;
pub const NO_BREAK_SPACE: u32 = 0xA0;
pub const ZERO_WIDTH_SPACE: u32 = 0x200B;
pub const ZERO_WIDTH_NON_JOINER: u32 = 0x200C;
pub const ZERO_WIDTH_JOINER: u32 = 0x200D;
pub const IDEOGRAPHIC_SPACE: u32 = 0x3000;
pub const VS15: u32 = 0xFE0E;
pub const VS16: u32 = 0xFE0F;
pub const LINE_SEPARATOR: u32 = 0x2028;
pub const PARAGRAPH_SEPARATOR: u32 = 0x2029;
pub const HYPHEN: u32 = 0x2010;
pub const NON_BREAKING_HYPHEN: u32 = 0x2011;

pub fn script(ch: u32) -> Script {
    CodePointMapData::<Script>::new().get32(ch)
}

pub fn script_extensions(ch: u32) -> Vec<Script> {
    ScriptWithExtensions::new().get_script_extensions_val32(ch).iter().collect()
}

pub fn general_category(ch: u32) -> GeneralCategory {
    CodePointMapData::<GeneralCategory>::new().get32(ch)
}

/// `U_GC_M_MASK | U_GC_LM_MASK | U_GC_SK_MASK`
pub fn is_mark_or_modifier(ch: u32) -> bool {
    let gc = general_category(ch);
    GeneralCategoryGroup::Mark.contains(gc)
        || gc == GeneralCategory::ModifierLetter
        || gc == GeneralCategory::ModifierSymbol
}

pub fn east_asian_width(ch: u32) -> EastAsianWidth {
    CodePointMapData::<EastAsianWidth>::new().get32(ch)
}

pub fn paired_bracket_type(ch: u32) -> BidiPairedBracketType {
    CodePointMapData::<BidiMirroringGlyph>::new().get32(ch).paired_bracket_type
}

pub fn paired_bracket(ch: u32) -> u32 {
    let data = CodePointMapData::<BidiMirroringGlyph>::new().get32(ch);
    if data.paired_bracket_type == BidiPairedBracketType::None {
        return ch;
    }
    data.mirroring_glyph.map(|c| c as u32).unwrap_or(ch)
}

pub fn bidi_class(ch: u32) -> BidiClass {
    CodePointMapData::<BidiClass>::new().get32(ch)
}

pub fn is_treat_as_space(c: u32) -> bool {
    c == SPACE || c == TAB || c == LINE_FEED || c == NO_BREAK_SPACE
}

pub fn is_default_ignorable(c: u32) -> bool {
    if c < 0x100 {
        return c == 0xAD;
    }
    CodePointSetData::new::<DefaultIgnorableCodePoint>().contains32(c)
}

pub fn treat_as_zero_width_space_in_complex_script(c: u32) -> bool {
    if c == 0x0C || c == 0x0D || c == 0xFFFC {
        return true;
    }
    is_default_ignorable(c)
}

pub fn treat_as_zero_width_space(c: u32) -> bool {
    treat_as_zero_width_space_in_complex_script(c) || c == ZERO_WIDTH_NON_JOINER || c == ZERO_WIDTH_JOINER
}

pub fn treat_as_zero_width_space_in_complex_script_legacy(c: u32) -> bool {
    c == 0x0C
        || c == 0x0D
        || c == 0xAD
        || c == ZERO_WIDTH_SPACE
        || (0x200E..=0x200F).contains(&c)
        || (0x202A..=0x202E).contains(&c)
        || c == 0xFEFF
        || c == 0xFFFC
}

pub fn is_normalized_canvas_space(c: u32) -> bool {
    c == 0x09 || (0x0A..=0x0D).contains(&c)
}

/// Bidi_Class R or AL, or a bidi control. Non-BMP may be bidi (Blink assumes so).
pub fn maybe_bidi_rtl(c: u32) -> bool {
    if c >= 0x10000 {
        return true;
    }
    if is_bidi_control(c) {
        return true;
    }
    matches!(bidi_class(c), BidiClass::RightToLeft | BidiClass::ArabicLetter)
}

pub fn is_bidi_control(c: u32) -> bool {
    c == 0x061C || c == 0x200E || c == 0x200F || (0x202A..=0x202E).contains(&c) || (0x2066..=0x2069).contains(&c)
}

const CJK_ARRAY: &[u32] = &[
    0x2C7, 0x2CA, 0x2CB, 0x2D9, 0x2020, 0x2021, 0x2030, 0x203B, 0x203C, 0x2042, 0x2047, 0x2048,
    0x2049, 0x2051, 0x20DD, 0x20DE, 0x2100, 0x2103, 0x2105, 0x2109, 0x210A, 0x2113, 0x2116, 0x2121,
    0x212B, 0x213B, 0x2150, 0x2151, 0x2152, 0x217F, 0x2189, 0x2307, 0x2312, 0x23CE, 0x2423, 0x25A0,
    0x25A1, 0x25A2, 0x25AA, 0x25AB, 0x25B1, 0x25B2, 0x25B3, 0x25B6, 0x25B7, 0x25BC, 0x25BD, 0x25C0,
    0x25C1, 0x25C6, 0x25C7, 0x25C9, 0x25CB, 0x25CC, 0x25EF, 0x2605, 0x2606, 0x260E, 0x2616, 0x2617,
    0x26A0, 0x2713, 0x271A, 0x273F, 0x2740, 0x2756, 0x2763, 0x2B1A, 0xFE10, 0xFE11, 0xFE12, 0xFE19,
    0xFF1D, 0x1F100, 0x1F200, 0x1F237, 0x1F32C, 0x1F336, 0x1F37D, 0x1F43F, 0x1F54F, 0x1F93B,
    0x1F946,
];

const CJK_RANGES: &[(u32, u32)] = &[
    (0x2E80, 0x2FDF), (0x31C0, 0x31EF), (0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF),
    (0x20000, 0x2FFFF), (0x2156, 0x215A), (0x2160, 0x216B), (0x2170, 0x217B), (0x23BE, 0x23CC),
    (0x2460, 0x2492), (0x249C, 0x24FF), (0x25CE, 0x25D3), (0x25E2, 0x25E6), (0x2600, 0x2603),
    (0x2660, 0x266F), (0x2672, 0x267D), (0x2776, 0x277F), (0x2FF0, 0x302D), (0x3031, 0x312F),
    (0x3190, 0x31BF), (0x3200, 0x33FF), (0x4DC0, 0x4DFF), (0xF860, 0xF862), (0xFE30, 0xFE6F),
    (0xFF00, 0xFF0C), (0xFF0E, 0xFF1A), (0xFF1F, 0xFFEF), (0x16FE0, 0x16FFF), (0x17000, 0x187FF),
    (0x18800, 0x18AFF), (0x1B000, 0x1B0FF), (0x1B100, 0x1B12F), (0x1B170, 0x1B2FF),
    (0x1F110, 0x1F129), (0x1F130, 0x1F149), (0x1F150, 0x1F169), (0x1F170, 0x1F189),
    (0x1F202, 0x1F219), (0x1F21B, 0x1F22E), (0x1F230, 0x1F231), (0x1F23B, 0x1F24F),
    (0x1F252, 0x1F2FF), (0x1F321, 0x1F32A), (0x1F394, 0x1F39F), (0x1F3CD, 0x1F3CE),
    (0x1F3D4, 0x1F3DF), (0x1F3F1, 0x1F3F2), (0x1F3F5, 0x1F3F7), (0x1F4FD, 0x1F4FE),
    (0x1F53E, 0x1F54A), (0x1F568, 0x1F573), (0x1F576, 0x1F579), (0x1F57B, 0x1F58F),
    (0x1F591, 0x1F594), (0x1F597, 0x1F5A3), (0x1F5A5, 0x1F5E7), (0x1F5E9, 0x1F5FA),
    (0x1F650, 0x1F67F), (0x1F6C6, 0x1F6CB), (0x1F6CD, 0x1F6CF), (0x1F6D3, 0x1F6D4),
    (0x1F6D9, 0x1F6DB), (0x1F6E0, 0x1F6EA), (0x1F6ED, 0x1F6F3), (0x1F6FD, 0x1F6FF),
    (0x1F900, 0x1F90B), (0x1FAC9, 0x1FACC),
];

/// Extended_Pictographic code points that occur inside RGI ZWJ or modifier
/// sequences but are not Emoji_Presentation (Blink derives this set from ICU's
/// RGI sequence data; this is the same set for Unicode 16).
const EXT_PICT_IN_RGI_SEQUENCES: &[u32] = &[
    0x2620, 0x2640, 0x2642, 0x2695, 0x2696, 0x2708, 0x2744, 0x2764, 0x26A7, 0x1F32B, 0x1F3F3,
    0x1F441, 0x1F5E8, 0x261D, 0x26F9, 0x270C, 0x270D, 0x1F3CB, 0x1F3CC, 0x1F574, 0x1F575,
    0x1F590, 0x2B1B, 0x1F525, 0x1F5E3,
];

pub fn is_cjk_ideograph_or_symbol(c: u32) -> bool {
    if c < 0x2C7 {
        return false;
    }
    if CJK_ARRAY.binary_search(&c).is_ok() || CJK_RANGES.iter().any(|&(a, b)| a <= c && c <= b) {
        return true;
    }
    if CodePointSetData::new::<EmojiPresentation>().contains32(c) {
        return true;
    }
    EXT_PICT_IN_RGI_SEQUENCES.contains(&c) || (is_emoji_modifier_base(c) && is_extended_pictographic(c))
}

pub fn is_cjk_ideograph_or_symbol_base(c: u32) -> bool {
    is_cjk_ideograph_or_symbol(c) && !is_mark_or_modifier(c)
}

pub fn is_ideographic(c: u32) -> bool {
    CodePointSetData::new::<Ideographic>().contains32(c)
}

pub fn is_emoji(c: u32) -> bool {
    CodePointSetData::new::<Emoji>().contains32(c)
}

pub fn is_emoji_emoji_default(c: u32) -> bool {
    CodePointSetData::new::<EmojiPresentation>().contains32(c)
}

pub fn is_emoji_text_default(c: u32) -> bool {
    is_emoji(c) && !is_emoji_emoji_default(c)
}

pub fn is_emoji_modifier_base(c: u32) -> bool {
    CodePointSetData::new::<EmojiModifierBase>().contains32(c)
}

pub fn is_emoji_component(c: u32) -> bool {
    CodePointSetData::new::<EmojiComponent>().contains32(c)
}

pub fn is_extended_pictographic(c: u32) -> bool {
    CodePointSetData::new::<ExtendedPictographic>().contains32(c)
}

pub fn is_emoji_reserved(c: u32) -> bool {
    general_category(c) == GeneralCategory::Unassigned && is_extended_pictographic(c)
}

pub fn is_emoji_including_reserved(c: u32) -> bool {
    is_emoji(c) || is_emoji_reserved(c)
}

pub fn is_emoji_keycap_base(c: u32) -> bool {
    (b'0' as u32..=b'9' as u32).contains(&c) || c == b'#' as u32 || c == b'*' as u32
}

pub fn is_regional_indicator(c: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&c)
}

pub fn is_modifier(c: u32) -> bool {
    (0x1F3FB..=0x1F3FF).contains(&c)
}

pub fn is_emoji_tag_sequence(c: u32) -> bool {
    (0xE0030..=0xE0039).contains(&c) || (0xE0061..=0xE007A).contains(&c)
}

pub fn maybe_emoji_presentation(c: u32) -> bool {
    if c < 0x80 {
        return c == 0xA9 || c == 0xAE || is_emoji_keycap_base(c);
    }
    c == ZERO_WIDTH_JOINER
        || (0x203C..=0x2B55).contains(&c)
        || c == VS15
        || c == 0x3030
        || c == 0x303D
        || c == 0x3297
        || c == 0x3299
        || c == VS16
        || c >= 0x10000
}

pub fn is_common_or_inherited_script(c: u32) -> bool {
    let s = script(c);
    s == Script::Common || s == Script::Inherited
}

pub fn has_likely_script(c: u32) -> bool {
    // ScriptBasedOnUnicodeBlock is not stable in Chromium 152.
    !is_common_or_inherited_script(c)
}

pub fn is_private_use(c: u32) -> bool {
    general_category(c) == GeneralCategory::PrivateUse
}

pub fn is_non_character(c: u32) -> bool {
    (0xFDD0..=0xFDEF).contains(&c) || (c & 0xFFFE) == 0xFFFE
}

pub fn is_unicode_variation_selector(c: u32) -> bool {
    (0x180B..=0x180D).contains(&c) || (0xFE00..=0xFE0F).contains(&c) || (0xE0100..=0xE01EF).contains(&c)
}

/// Approximates Blink's `IsVariationSequence`: emoji variation sequences
/// (VS15 or VS16 after an Emoji character) and ideographic variation
/// sequences. Standardized variants for math and Mongolian are not modelled.
pub fn is_variation_sequence(base: u32, vs: u32) -> bool {
    if vs == VS15 || vs == VS16 {
        return is_emoji(base);
    }
    if (0xE0100..=0xE01EF).contains(&vs) {
        return is_ideographic(base);
    }
    false
}

/// Emoji segmentation categories (`emoji_segmentation_category.h`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmojiCat {
    Emoji,
    EmojiTextPresentation,
    EmojiEmojiPresentation,
    EmojiModifierBase,
    EmojiModifier,
    RegionalIndicator,
    KeycapBase,
    CombiningEnclosingKeycap,
    CombiningEnclosingCircleBackslash,
    Zwj,
    Vs15,
    Vs16,
    TagBase,
    TagSequence,
    TagTerm,
    Other,
}

pub fn emoji_category(c: u32) -> EmojiCat {
    if c <= 0x7F {
        return if is_emoji_keycap_base(c) { EmojiCat::KeycapBase } else { EmojiCat::Other };
    }
    match c {
        0x20E3 => return EmojiCat::CombiningEnclosingKeycap,
        0x20E0 => return EmojiCat::CombiningEnclosingCircleBackslash,
        ZERO_WIDTH_JOINER => return EmojiCat::Zwj,
        VS15 => return EmojiCat::Vs15,
        VS16 => return EmojiCat::Vs16,
        0x1F3F4 => return EmojiCat::TagBase,
        0xE007F => return EmojiCat::TagTerm,
        _ => {}
    }
    if is_emoji_tag_sequence(c) {
        return EmojiCat::TagSequence;
    }
    if is_emoji_modifier_base(c) {
        return EmojiCat::EmojiModifierBase;
    }
    if is_modifier(c) {
        return EmojiCat::EmojiModifier;
    }
    if is_regional_indicator(c) {
        return EmojiCat::RegionalIndicator;
    }
    if is_emoji_emoji_default(c) {
        return EmojiCat::EmojiEmojiPresentation;
    }
    if is_emoji_text_default(c) {
        return EmojiCat::EmojiTextPresentation;
    }
    if is_emoji_including_reserved(c) {
        return EmojiCat::Emoji;
    }
    EmojiCat::Other
}

/// JavaScript `\s` (WhiteSpace and LineTerminator), used by the legacy layout.
pub fn is_js_space(u: u16) -> bool {
    matches!(
        u,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}
