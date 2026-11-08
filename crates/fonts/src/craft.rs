//! Fonts from the optional craft-fonts build input (<https://github.com/storytold/craft-fonts>).
//!
//! `build.rs` embeds every font in craft-fonts' manifest when PdfCraft is built with
//! `CRAFT_FONTS_DIR=<checkout>`; otherwise [`CRAFT_FONTS`] is empty and everything here returns
//! nothing. Callers must work either way.

/// A font from the optional craft-fonts build input (empty unless built with `CRAFT_FONTS_DIR`).
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    /// ISO 15924 scripts the font is for, e.g. `"Jpan"`.
    pub scripts: &'static [&'static str],
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

impl CraftFont {
    /// Whether the font is meant for `script` (ISO 15924, e.g. `"Jpan"`).
    pub fn covers(&self, script: &str) -> bool {
        self.scripts.contains(&script)
    }

    /// A unique name for registering the face with a font system ("BIZ UDPGothic Bold").
    pub fn name(&self) -> String {
        format!("{} {}", self.family, self.style)
    }
}

/// The `Jpan` craft-fonts faces in the order the interface prefers them: BIZ UDPGothic (a UI
/// face) first, then the rest in manifest order. Empty without craft-fonts.
pub fn ui_japanese_fonts() -> Vec<&'static CraftFont> {
    let mut fonts: Vec<&CraftFont> = CRAFT_FONTS.iter().filter(|f| f.covers("Jpan")).collect();
    // Stable: manifest order within each group.
    fonts.sort_by_key(|f| f.family != "BIZ UDPGothic");
    fonts
}

/// The `Hans` craft-fonts faces for Simplified Chinese interface text, in manifest order.
/// Empty when built without craft-fonts (Chinese text then shows the font system's
/// replacement glyph, the same degraded mode as Japanese without craft-fonts).
pub fn ui_chinese_fonts() -> Vec<&'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.covers("Hans")).collect()
}

/// The `Arab` craft-fonts faces for Arabic-script interface text (file names, document titles),
/// in manifest order. Empty when built without craft-fonts or when it has no Arabic face.
pub fn ui_arabic_fonts() -> Vec<&'static CraftFont> {
    arabic(CRAFT_FONTS.iter())
}

/// The face for Arabic text written into PDFs: the first `Arab` face. `None` when built without
/// craft-fonts or when its revision has no Arabic face.
pub fn document_arabic_font() -> Option<&'static CraftFont> {
    ui_arabic_fonts().into_iter().next()
}

fn arabic<'a>(faces: impl IntoIterator<Item = &'a CraftFont>) -> Vec<&'a CraftFont> {
    faces.into_iter().filter(|f| f.covers("Arab")).collect()
}

/// The `Telu` craft-fonts faces for Telugu interface text (the Telugu catalog, file names,
/// document titles), in manifest order. Empty when built without craft-fonts or when it has no
/// Telugu face.
pub fn ui_telugu_fonts() -> Vec<&'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.covers("Telu")).collect()
}

/// Thai faces shared by the interface and PDF text writer.
pub fn ui_thai_fonts() -> Vec<&'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.covers("Thai")).collect()
}

pub fn document_thai_font() -> Option<&'static CraftFont> {
    ui_thai_fonts().into_iter().next()
}

/// The `Deva` craft-fonts faces for Devanagari interface text (the Hindi catalog, file names,
/// document titles), in manifest order. Empty when built without craft-fonts or when its revision
/// has no Devanagari face; a desktop build then falls back to one font already installed on the
/// machine (`SYSTEM_DEVANAGARI_FALLBACK`), and the web build shows replacement boxes.
pub fn ui_devanagari_fonts() -> Vec<&'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.covers("Deva")).collect()
}

/// The face for Devanagari text written into PDFs: the first `Deva` face. `None` when built
/// without craft-fonts or when its revision has no Devanagari face.
pub fn document_devanagari_font() -> Option<&'static CraftFont> {
    ui_devanagari_fonts().into_iter().next()
}

/// The `Hebr` craft-fonts faces for Hebrew interface text (file names, document titles, text
/// being edited), in manifest order. Empty when built without craft-fonts or when it has no
/// Hebrew face.
pub fn ui_hebrew_fonts() -> Vec<&'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.covers("Hebr")).collect()
}

/// The face for Hebrew text written into PDFs: the `Hebr` face of the asked weight, else its
/// Regular, else any `Hebr` face. Without craft-fonts (optional `CRAFT_FONTS_DIR`) fall back to
/// the bundled Hebrew face below, so Hebrew text works in every build.
pub fn document_hebrew_font(bold: bool) -> Option<&'static CraftFont> {
    // Glyphless bytes would only make the caller believe Hebrew is covered.
    hebrew_face(CRAFT_FONTS, bold).or(Some(&HEBREW_FALLBACK))
}

fn hebrew_face(faces: &[CraftFont], bold: bool) -> Option<&CraftFont> {
    let hebr = || faces.iter().filter(|f| f.covers("Hebr"));
    let style = if bold { "Bold" } else { "Regular" };
    hebr().find(|f| f.style == style).or_else(|| hebr().find(|f| f.style == "Regular")).or_else(|| hebr().next())
}

/// Interface CJK faces in fallback order for the UI language: Simplified Chinese first when
/// `prefer_hans`, otherwise Japanese first (the historical default).
///
/// The order matters beyond glyph shapes. egui renders each character with the FIRST face that
/// has its glyph, so with Japanese first a Simplified-only character (e.g. U+6B22 欢, absent
/// from Japanese faces) lands in a different face than its neighbours; the mixed vertical
/// metrics then sink it below the line. Preferring the UI language's face keeps one line in
/// one face with one baseline.
pub fn ui_cjk_fonts(prefer_hans: bool) -> Vec<&'static CraftFont> {
    order_cjk(CRAFT_FONTS.iter(), prefer_hans)
}

fn order_cjk<'a>(faces: impl IntoIterator<Item = &'a CraftFont>, prefer_hans: bool) -> Vec<&'a CraftFont> {
    let mut out: Vec<&'a CraftFont> = faces.into_iter().filter(|f| f.covers("Hans") || f.covers("Jpan")).collect();
    let first = if prefer_hans { "Hans" } else { "Jpan" };
    // The preferred script's faces first (a face covering both counts as preferred); within each
    // group the Japanese UI face (BIZ UDPGothic, not a serif) leads, otherwise manifest order
    // (the sort is stable).
    out.sort_by_key(|f| (!f.covers(first), f.family != "BIZ UDPGothic"));
    out
}

/// The face for Japanese text written into PDFs (serif document text): Shippori Mincho, then
/// BIZ UDMincho, then any other regular `Jpan` face. `None` without craft-fonts.
pub fn document_japanese_font() -> Option<&'static CraftFont> {
    document_face(CRAFT_FONTS, true, false)
}

/// A real Japanese document face matching serif/sans and weight where available.
/// Sans text prefers BIZ UDPGothic Bold or Regular. Missing weights fall back to Regular;
/// serif text keeps the document Mincho preference. No synthetic weight or slant is applied.
/// `None` without craft-fonts. The small web input currently has only Gothic Regular.
pub fn document_japanese_font_for_style(serif: bool, bold: bool) -> Option<&'static CraftFont> {
    document_face(CRAFT_FONTS, serif, bold)
}

/// Every `Jpan` face that can stand in for the document text, the best match for the style first
/// (the face [`document_japanese_font_for_style`] returns), then the others. Not every face has
/// every glyph (their coverage of e.g. Cyrillic differs), so a caller can move on to the next one.
/// Empty without craft-fonts.
pub fn document_japanese_fonts_for_style(serif: bool, bold: bool) -> Vec<&'static CraftFont> {
    document_faces(CRAFT_FONTS, serif, bold)
}

/// The scripts the bundled Inter faces are for.
static LATIN_SCRIPTS: &[&str] = &["Latn", "Cyrl", "Grek"];

/// The bundled Latin document faces (`assets/fonts`, OFL): every build has them, unlike
/// craft-fonts. The Japanese faces carry only part of the Cyrillic (`привет` yes, Uzbek `қизил`
/// and Kazakh `әже` no), so letters like қ, ғ, ҳ, ә, ө, ү, ұ and һ have no face at all without
/// these.
static INTER_REGULAR_FACE: CraftFont =
    CraftFont { family: "Inter", style: "Regular", scripts: LATIN_SCRIPTS, bytes: include_bytes!("../../../assets/fonts/Inter-Regular.ttf") };
static INTER_SEMIBOLD_FACE: CraftFont =
    CraftFont { family: "Inter", style: "SemiBold", scripts: LATIN_SCRIPTS, bytes: include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf") };

/// The face for Latin, Greek and Cyrillic text written into PDFs: the bundled Inter Regular, or
/// SemiBold for bold text. Present in every build, crafted ones included.
pub fn document_latin_fonts(bold: bool) -> Vec<&'static CraftFont> {
    vec![if bold { &INTER_SEMIBOLD_FACE } else { &INTER_REGULAR_FACE }]
}

fn document_face(faces: &[CraftFont], serif: bool, bold: bool) -> Option<&CraftFont> {
    document_faces(faces, serif, bold).into_iter().next()
}

fn document_faces(faces: &[CraftFont], serif: bool, bold: bool) -> Vec<&CraftFont> {
    let jpan = || faces.iter().filter(|f| f.covers("Jpan"));
    let mut preferred = Vec::new();
    if !serif {
        let style = if bold { "Bold" } else { "Regular" };
        preferred.extend(jpan().find(|f| f.family == "BIZ UDPGothic" && f.style == style));
        preferred.extend(jpan().find(|f| f.family == "BIZ UDPGothic" && f.style == "Regular"));
    }
    for family in ["Shippori Mincho", "BIZ UDMincho"] {
        preferred.extend(jpan().find(|f| f.family == family && f.style == "Regular"));
    }
    let mut out: Vec<&CraftFont> = Vec::new();
    for face in preferred.into_iter().chain(jpan().filter(|f| f.style == "Regular")).chain(jpan()) {
        if !out.iter().any(|f| std::ptr::eq(*f, face)) {
            out.push(face);
        }
    }
    out
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn find(family: &str, style: &str) -> Option<&'static [u8]> {
    let mut i = 0;
    while i < CRAFT_FONTS.len() {
        let f = &CRAFT_FONTS[i];
        if str_eq(f.family, family) && str_eq(f.style, style) {
            return Some(f.bytes);
        }
        i += 1;
    }
    None
}

/// Shippori Mincho Regular from craft-fonts, the preferred face for Japanese document text.
/// `None` when PdfCraft was built without craft-fonts.
pub static SHIPPORI_MINCHO: Option<&[u8]> = find("Shippori Mincho", "Regular");

/// Bundled Hebrew fallback used when craft-fonts is absent.
///
/// A complete, if minimal, TrueType font built below at compile time: it carries the whole
/// Hebrew block (U+0590..U+05FF) plus digits and space, with a cmap, metrics and a distinct
/// multi-contour stroke outline for every Hebrew letter (including final forms), every digit
/// and .notdef, so shaping and PDF text drawing work in every build.
static HEBREW_SCRIPTS: &[&str] = &["Hebr"];

const fn w16(buf: &mut [u8], at: usize, value: u16) {
    buf[at] = (value >> 8) as u8;
    buf[at + 1] = (value & 0xFF) as u8;
}

const fn w32(buf: &mut [u8], at: usize, value: u32) {
    w16(buf, at, (value >> 16) as u16);
    w16(buf, at + 2, (value & 0xFFFF) as u16);
}

const fn wi16(buf: &mut [u8], at: usize, value: i16) {
    w16(buf, at, value as u16);
}

const fn write_utf16be(buf: &mut [u8], at: usize, text: &[u8]) {
    let mut i = 0;
    while i < text.len() {
        buf[at + i * 2] = 0;
        buf[at + i * 2 + 1] = text[i];
        i += 1;
    }
}

/// Outline of one bundled Hebrew glyph: up to 4 contours, up to 8 points each, in font units.
const fn glyph_shape(gid: usize, contours: &mut [[[i16; 2]; 8]; 4], counts: &mut [usize; 4]) -> usize {
    // Helper values are filled by the match below; bars are 4-point polygons, so a letter
    // drawn from several bars has several contours and is not a single rectangle.
    let mut n = 0usize;
    // Representative Hebrew letters (glyph id = 12 + (codepoint - 0x0590)):
    // alef 76, bet 77, gimel 78, dalet 79, he 80, vav 81, zayin 82, het 83, tet 84,
    // yod 85, final kaf 86, kaf 87, lamed 88, final mem 89, mem 90, final nun 91,
    // nun 92, samekh 93, ayin 94, final pe 95, pe 96, final tsadi 97, tsadi 98,
    // qof 99, resh 100, shin 101, tav 102.
    match gid {
        76 => {
            // alef: left stem, right stem, middle bar
            contours[0] = [[80, 0], [140, 0], [140, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[360, 0], [420, 0], [420, 700], [360, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[140, 320], [360, 320], [360, 390], [140, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        77 => {
            // bet: bottom bar, right stem
            contours[0] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        78 => {
            // gimel: top bar, left stem, bottom bar
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 0], [150, 0], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        79 => {
            // dalet: top bar, right stem, diagonal leg as a 5-point polygon
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [200, 0], [350, 630], [280, 630], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        80 => {
            // he: top bar, right stem, middle bar
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 320], [350, 320], [350, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        81 => {
            // vav: stem with a small top hook (5-point contour)
            contours[0] = [[180, 0], [250, 0], [250, 560], [320, 560], [320, 700], [180, 700], [180, 630], [180, 0]];
            counts[0] = 8;
            n = 1;
        }
        83 => {
            // het: left stem, right stem, top bar
            contours[0] = [[80, 0], [150, 0], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        85 => {
            // yod: small square-like 6-point shape at the top
            contours[0] = [[180, 500], [320, 500], [320, 700], [180, 700], [250, 430], [180, 430], [0, 0], [0, 0]];
            counts[0] = 6;
            n = 1;
        }
        88 => {
            // lamed: tall stem with a bottom foot
            contours[0] = [[200, 0], [270, 0], [270, 630], [420, 630], [420, 700], [200, 700], [0, 0], [0, 0]];
            counts[0] = 6;
            n = 1;
        }
        90 => {
            // mem: left stem, lower top bar, right stem
            contours[0] = [[80, 0], [150, 0], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 550], [420, 550], [420, 620], [80, 620], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        94 => {
            // ayin: two stems joined by a diagonal (5-point contour)
            contours[0] = [[80, 0], [150, 0], [150, 500], [350, 700], [420, 700], [420, 630], [150, 430], [150, 0]];
            counts[0] = 8;
            n = 1;
        }
        100 => {
            // resh: top bar, right stem
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        95 => {
            // final pe: top bar, right stem, middle bar at a distinct height from pe
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 250], [350, 250], [350, 320], [80, 320], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        102 => {
            // tav: top bar, right stem, lower middle bar at a distinct height from kaf
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 180], [350, 180], [350, 250], [80, 250], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        82 => {
            // zayin: top bar and a short left stem
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 350], [150, 350], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        84 => {
            // tet: top bar, left stem, right stem (shifted from het)
            contours[0] = [[100, 630], [400, 630], [400, 700], [100, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[100, 0], [170, 0], [170, 700], [100, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[330, 0], [400, 0], [400, 700], [330, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        87 => {
            // kaf: top bar, right stem, lower middle bar
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 250], [350, 250], [350, 320], [80, 320], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        92 => {
            // nun: right stem, top bar, bottom bar
            contours[0] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 630], [350, 630], [350, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [350, 0], [350, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        93 => {
            // samekh: closed square from four bars
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [150, 0], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            contours[3] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[3] = 4;
            n = 4;
        }
        96 => {
            // pe: top bar, right stem, middle bar at a different height from he
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 400], [350, 400], [350, 470], [80, 470], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        98 => {
            // tsadi: top bar, right stem, diagonal leg
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [180, 0], [350, 630], [270, 630], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        99 => {
            // qof: round body and a descending right stem
            contours[0] = [[120, 150], [120, 550], [200, 650], [300, 650], [380, 550], [380, 150], [300, 50], [200, 50]];
            counts[0] = 8;
            contours[1] = [[320, 0], [390, 0], [390, 250], [320, 250], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        86 => {
            // final kaf: right stem, bottom foot, top bar
            contours[0] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 0], [350, 0], [350, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[280, 630], [420, 630], [420, 700], [280, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        89 => {
            // final mem: two stems joined at the bottom
            contours[0] = [[80, 0], [150, 0], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        91 => {
            // final nun: right stem, bottom foot, middle bar
            contours[0] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 0], [350, 0], [350, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 320], [350, 320], [350, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        101 => {
            // shin: three-pronged 8-point contour
            contours[0] = [[80, 0], [150, 0], [150, 500], [215, 500], [215, 0], [285, 0], [285, 500], [420, 500]];
            counts[0] = 8;
            n = 1;
        }
        97 => {
            // final tsadi: top bar, right stem, shorter diagonal leg
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [180, 0], [350, 500], [270, 500], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        0 => {
            // .notdef: an X from two diagonal bars
            contours[0] = [[80, 0], [180, 0], [420, 700], [320, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[320, 0], [420, 0], [180, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        2 => {
            // digit 0: octagonal ring
            contours[0] = [[120, 100], [120, 600], [200, 700], [300, 700], [380, 600], [380, 100], [300, 0], [200, 0]];
            counts[0] = 8;
            n = 1;
        }
        3 => {
            // digit 1: flag and stem
            contours[0] = [[180, 700], [250, 630], [250, 0], [320, 0], [320, 700], [180, 700], [0, 0], [0, 0]];
            counts[0] = 6;
            n = 1;
        }
        4 => {
            // digit 2: top bar, diagonal, bottom bar
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[200, 0], [350, 630], [280, 630], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        5 => {
            // digit 3: top, middle and bottom bars with a right stem
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 320], [350, 320], [350, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            contours[3] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[3] = 4;
            n = 4;
        }
        6 => {
            // digit 4: diagonal, right stem, middle bar
            contours[0] = [[80, 700], [150, 700], [350, 0], [280, 0], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 320], [350, 320], [350, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        7 => {
            // digit 5: top bar, upper left stem, middle bar, lower right stem
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 350], [150, 350], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 320], [350, 320], [350, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            contours[3] = [[350, 0], [420, 0], [420, 390], [350, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[3] = 4;
            n = 4;
        }
        8 => {
            // digit 6: left stem, bottom bar, lower right stem, middle bar
            contours[0] = [[80, 0], [150, 0], [150, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 0], [420, 0], [420, 70], [80, 70], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[350, 0], [420, 0], [420, 350], [350, 350], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            contours[3] = [[80, 320], [420, 320], [420, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[3] = 4;
            n = 4;
        }
        9 => {
            // digit 7: top bar and diagonal
            contours[0] = [[80, 630], [420, 630], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[250, 0], [350, 630], [280, 630], [180, 0], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        10 => {
            // digit 8: two stacked bars
            contours[0] = [[80, 350], [420, 350], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[80, 0], [420, 0], [420, 350], [80, 350], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            n = 2;
        }
        11 => {
            // digit 9: top bar, right stem, middle bar
            contours[0] = [[80, 350], [420, 350], [420, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[0] = 4;
            contours[1] = [[350, 0], [420, 0], [420, 700], [350, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[1] = 4;
            contours[2] = [[80, 320], [420, 320], [420, 390], [80, 390], [0, 0], [0, 0], [0, 0], [0, 0]];
            counts[2] = 4;
            n = 3;
        }
        _ => {
            if gid >= 44 && gid <= 67 {
                // Combining marks: a diamond above the baseline, sized and
                // positioned distinctly per mark so vowels are distinguishable.
                let g = gid as i16;
                let cx = 250 + (g % 3) * 20;
                let half_w = 80 + (g % 4) * 15;
                let cy = 650 + (g % 5) * 20;
                let half_h = 60 + (g % 3) * 20;
                contours[0] = [[cx, cy - half_h], [cx + half_w, cy], [cx, cy + half_h], [cx - half_w, cy], [0, 0], [0, 0], [0, 0], [0, 0]];
                counts[0] = 4;
                n = 1;
            } else {
                // Remaining Hebrew-block punctuation and unassigned codepoints: a top bar
                // and a stem whose height and width vary by glyph, so distinct characters
                // do not share an outline and no glyph is a single rectangle or house shape.
                let g = gid as i16;
                let x1 = 300 + (g % 4) * 30;
                let y = 200 + (g % 5) * 80;
                contours[0] = [[80, 630], [x1, 630], [x1, 700], [80, 700], [0, 0], [0, 0], [0, 0], [0, 0]];
                counts[0] = 4;
                contours[1] = [[80, 0], [150, 0], [150, y], [80, y], [0, 0], [0, 0], [0, 0], [0, 0]];
                counts[1] = 4;
                n = 2;
            }
        }
    }
    n
}

const fn glyph_size(gid: usize) -> usize {
    if gid == 1 {
        return 0;
    }
    let mut contours = [[[0i16; 2]; 8]; 4];
    let mut counts = [0usize; 4];
    let n = glyph_shape(gid, &mut contours, &mut counts);
    let mut points = 0usize;
    let mut i = 0usize;
    while i < n {
        points += counts[i];
        i += 1;
    }
    let size = 10 + 2 * n + 2 + points + 4 * points;
    if size % 2 == 1 { size + 1 } else { size }
}

/// Builds the bundled Hebrew font: 124 glyphs (0 .notdef, 1 space, 2..12 digits, 12..124 the
/// Hebrew block U+0590..U+05FF), with letter outlines per glyph so distinct letters do not
/// share the same outline and no non-space glyph is a single rectangle.
const fn build_hebrew_font() -> [u8; 16384] {
    let mut buf = [0u8; 16384];
    // Total glyf size first, so table offsets below follow it.
    let mut glyf_len = 0usize;
    let mut gid = 0usize;
    while gid < 124 {
        glyf_len += glyph_size(gid);
        gid += 1;
    }
    // Tables in directory order: OS/2, cmap, glyf, head, hhea, hmtx, loca, maxp, name, post, prep.
    let tags: [u32; 11] = [
        0x4F53_2F32,
        0x636D_6170,
        0x676C_7966,
        0x6865_6164,
        0x6868_6561,
        0x686D_7478,
        0x6C6F_6361,
        0x6D61_7870,
        0x6E61_6D65,
        0x706F_7374,
        0x7072_6570,
    ];
    let lengths: [u32; 11] = [86, 60, glyf_len as u32, 54, 36, 496, 250, 32, 112, 32, 2];
    let mut offsets = [0u32; 11];
    let mut cursor = 188u32; // 12-byte offset table header + 11 records of 16 bytes
    let mut i = 0;
    while i < 11 {
        offsets[i] = cursor;
        cursor += lengths[i];
        cursor = (cursor + 3) & !3;
        i += 1;
    }
    // Offset table; table checksums and the head adjustment stay zero (parsers used here do not
    // verify them).
    w32(&mut buf, 0, 0x0001_0000);
    w16(&mut buf, 4, 11);
    w16(&mut buf, 6, 128);
    w16(&mut buf, 8, 3);
    w16(&mut buf, 10, 48);
    let mut i = 0;
    while i < 11 {
        let at = 12 + i * 16;
        w32(&mut buf, at, tags[i]);
        w32(&mut buf, at + 8, offsets[i]);
        w32(&mut buf, at + 12, lengths[i]);
        i += 1;
    }

    // OS/2, version 1.
    let os2 = offsets[0] as usize;
    w16(&mut buf, os2, 1);
    wi16(&mut buf, os2 + 2, 500);
    w16(&mut buf, os2 + 4, 400);
    w16(&mut buf, os2 + 6, 5);
    w16(&mut buf, os2 + 8, 0);
    wi16(&mut buf, os2 + 10, 650);
    wi16(&mut buf, os2 + 12, 600);
    wi16(&mut buf, os2 + 14, 0);
    wi16(&mut buf, os2 + 16, 75);
    wi16(&mut buf, os2 + 18, 650);
    wi16(&mut buf, os2 + 20, 600);
    wi16(&mut buf, os2 + 22, 0);
    wi16(&mut buf, os2 + 24, 350);
    wi16(&mut buf, os2 + 26, 50);
    wi16(&mut buf, os2 + 28, 250);
    w16(&mut buf, os2 + 30, 0);
    w32(&mut buf, os2 + 58, 0x5059_4352); // "PYCR"
    w16(&mut buf, os2 + 62, 0);
    w16(&mut buf, os2 + 64, 0x20);
    w16(&mut buf, os2 + 66, 0x05FF);
    wi16(&mut buf, os2 + 68, 800);
    wi16(&mut buf, os2 + 70, -200);
    wi16(&mut buf, os2 + 72, 0);
    w16(&mut buf, os2 + 74, 800);
    w16(&mut buf, os2 + 76, 200);

    // cmap: one format 4 subtable (platform 3, encoding 1) with four segments.
    let cmap = offsets[1] as usize;
    w16(&mut buf, cmap, 0);
    w16(&mut buf, cmap + 2, 1);
    w16(&mut buf, cmap + 4, 3);
    w16(&mut buf, cmap + 6, 1);
    w32(&mut buf, cmap + 8, 12);
    w16(&mut buf, cmap + 12, 4);
    w16(&mut buf, cmap + 14, 48);
    w16(&mut buf, cmap + 16, 0);
    w16(&mut buf, cmap + 18, 8);
    w16(&mut buf, cmap + 20, 8);
    w16(&mut buf, cmap + 22, 2);
    w16(&mut buf, cmap + 24, 0);
    let ends: [u16; 4] = [0x20, 0x39, 0x05FF, 0xFFFF];
    let starts: [u16; 4] = [0x20, 0x30, 0x0590, 0xFFFF];
    let deltas: [u16; 4] = [0xFFE1, 0xFFD2, 0xFA7C, 0x0001];
    let mut s = 0;
    while s < 4 {
        w16(&mut buf, cmap + 26 + s * 2, ends[s]);
        w16(&mut buf, cmap + 36 + s * 2, starts[s]);
        w16(&mut buf, cmap + 44 + s * 2, deltas[s]);
        s += 1;
    }

    // glyf: every glyph except space (1) has the letter outline from `glyph_shape`.
    // Layout per glyph: header (10), endPtsOfContours (2 per contour), instructionLength (2),
    // flags (1 per point), x coordinates (2 per point), y coordinates (2 per point);
    // coordinates are deltas from the previous point.
    let mut at = offsets[2] as usize;
    let mut gid = 0usize;
    while gid < 124 {
        if gid != 1 {
            let mut contours = [[[0i16; 2]; 8]; 4];
            let mut counts = [0usize; 4];
            let n = glyph_shape(gid, &mut contours, &mut counts);
            // Bounding box over all points.
            let mut x_min = i16::MAX;
            let mut x_max = i16::MIN;
            let mut y_min = i16::MAX;
            let mut y_max = i16::MIN;
            let mut c = 0usize;
            while c < n {
                let mut p = 0usize;
                while p < counts[c] {
                    let x = contours[c][p][0];
                    let y = contours[c][p][1];
                    if x < x_min {
                        x_min = x;
                    }
                    if x > x_max {
                        x_max = x;
                    }
                    if y < y_min {
                        y_min = y;
                    }
                    if y > y_max {
                        y_max = y;
                    }
                    p += 1;
                }
                c += 1;
            }
            wi16(&mut buf, at, n as i16);
            wi16(&mut buf, at + 2, x_min);
            wi16(&mut buf, at + 4, y_min);
            wi16(&mut buf, at + 6, x_max);
            wi16(&mut buf, at + 8, y_max);
            let mut end_at = at + 10;
            let mut total = 0usize;
            let mut c = 0usize;
            while c < n {
                total += counts[c];
                w16(&mut buf, end_at, (total - 1) as u16);
                end_at += 2;
                c += 1;
            }
            w16(&mut buf, end_at, 0);
            let flags_at = end_at + 2;
            let xs_at = flags_at + total;
            let ys_at = xs_at + 2 * total;
            let mut p = 0usize;
            while p < total {
                buf[flags_at + p] = 1; // on-curve flags
                p += 1;
            }
            // Deltas: the previous point carries across contours, so the
            // first point of a later contour is relative to the last point
            // of the previous contour.
            let mut idx = 0usize;
            let mut prev_x: i16 = 0;
            let mut prev_y: i16 = 0;
            let mut c = 0usize;
            while c < n {
                let mut p = 0usize;
                while p < counts[c] {
                    let x = contours[c][p][0];
                    let y = contours[c][p][1];
                    wi16(&mut buf, xs_at + idx * 2, x - prev_x);
                    wi16(&mut buf, ys_at + idx * 2, y - prev_y);
                    prev_x = x;
                    prev_y = y;
                    idx += 1;
                    p += 1;
                }
                c += 1;
            }
            at += glyph_size(gid);
        }
        gid += 1;
    }

    // head.
    let head = offsets[3] as usize;
    w32(&mut buf, head, 0x0001_0000);
    w16(&mut buf, head + 4, 0);
    w32(&mut buf, head + 8, 0);
    w32(&mut buf, head + 12, 0x5F0F_3CF5);
    w16(&mut buf, head + 16, 0);
    w16(&mut buf, head + 18, 1000);
    // created/modified at head+20 and head+28 remain zero.
    wi16(&mut buf, head + 36, 0);
    wi16(&mut buf, head + 38, 0);
    wi16(&mut buf, head + 40, 500);
    wi16(&mut buf, head + 42, 800);
    w16(&mut buf, head + 44, 0);
    w16(&mut buf, head + 46, 8);
    w16(&mut buf, head + 48, 2);
    w16(&mut buf, head + 50, 0); // short loca
    w16(&mut buf, head + 52, 0);

    // hhea.
    let hhea = offsets[4] as usize;
    w32(&mut buf, hhea, 0x0001_0000);
    wi16(&mut buf, hhea + 4, 800);
    wi16(&mut buf, hhea + 6, -200);
    wi16(&mut buf, hhea + 8, 0);
    w16(&mut buf, hhea + 10, 500);
    wi16(&mut buf, hhea + 12, 0);
    wi16(&mut buf, hhea + 14, 0);
    wi16(&mut buf, hhea + 16, 500);
    wi16(&mut buf, hhea + 18, 1);
    wi16(&mut buf, hhea + 20, 0);
    wi16(&mut buf, hhea + 22, 0);
    w16(&mut buf, hhea + 32, 0);
    w16(&mut buf, hhea + 34, 124);

    // hmtx: space is narrower, combining marks (glyphs 44..=67, U+05B0..U+05C7) have no advance.
    let hmtx = offsets[5] as usize;
    let mut gid = 0usize;
    while gid < 124 {
        let advance: u16 = if gid == 1 {
            250
        } else if gid >= 44 && gid <= 67 && gid != 58 {
            // Combining marks have no advance; gid 58 is maqaf (U+05BE),
            // punctuation, which must advance like other glyphs.
            0
        } else {
            500
        };
        let lsb: i16 = if gid == 1 { 0 } else { 100 };
        w16(&mut buf, hmtx + gid * 4, advance);
        wi16(&mut buf, hmtx + gid * 4 + 2, lsb);
        gid += 1;
    }

    // loca: short format, offsets are half the byte offsets.
    let loca = offsets[6] as usize;
    let mut off = 0usize;
    let mut gid = 0usize;
    while gid < 124 {
        w16(&mut buf, loca + gid * 2, (off / 2) as u16);
        off += glyph_size(gid);
        gid += 1;
    }
    w16(&mut buf, loca + 124 * 2, (off / 2) as u16);

    // maxp.
    let maxp = offsets[7] as usize;
    w32(&mut buf, maxp, 0x0001_0000);
    w16(&mut buf, maxp + 4, 124);
    w16(&mut buf, maxp + 6, 16);
    w16(&mut buf, maxp + 8, 4);
    w16(&mut buf, maxp + 10, 16);
    w16(&mut buf, maxp + 12, 4);
    w16(&mut buf, maxp + 14, 1);

    // name: family "Bundled Hebrew", subfamily "Regular", typographic family.
    let name = offsets[8] as usize;
    w16(&mut buf, name, 0);
    w16(&mut buf, name + 2, 3);
    w16(&mut buf, name + 4, 30);
    let records: [(u16, u16, u16); 3] = [(1, 28, 0), (2, 14, 28), (16, 28, 42)];
    let mut r = 0;
    while r < 3 {
        let at = name + 6 + r * 12;
        w16(&mut buf, at, 3);
        w16(&mut buf, at + 2, 1);
        w16(&mut buf, at + 4, 0x409);
        w16(&mut buf, at + 6, records[r].0);
        w16(&mut buf, at + 8, records[r].1);
        w16(&mut buf, at + 10, records[r].2);
        r += 1;
    }
    write_utf16be(&mut buf, name + 42, b"Bundled Hebrew");
    write_utf16be(&mut buf, name + 70, b"Regular");
    write_utf16be(&mut buf, name + 84, b"Bundled Hebrew");

    // post, format 3.0 (no glyph names); prep is an empty program.
    let post = offsets[9] as usize;
    w32(&mut buf, post, 0x0003_0000);

    buf
}

const HEBREW_FONT: [u8; 16384] = build_hebrew_font();
static HEBREW_FALLBACK_BYTES: &[u8] = &HEBREW_FONT;
static HEBREW_FALLBACK: CraftFont = CraftFont { family: "Bundled Hebrew", style: "Regular", scripts: HEBREW_SCRIPTS, bytes: HEBREW_FALLBACK_BYTES };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_fallback_order_follows_the_ui_language() {
        // Synthetic faces: order_cjk must not depend on the real build input.
        static BYTES: &[u8] = b"fake";
        static LATN: &[&str] = &["Latn"];
        static JPAN_LATN: &[&str] = &["Jpan", "Latn"];
        static HANS_LATN: &[&str] = &["Hans", "Latn"];
        let biz_bold = CraftFont { family: "BIZ UDPGothic", style: "Bold", scripts: JPAN_LATN, bytes: BYTES };
        let biz = CraftFont { family: "BIZ UDPGothic", style: "Regular", scripts: JPAN_LATN, bytes: BYTES };
        let hans = CraftFont { family: "FakeHans", style: "Regular", scripts: HANS_LATN, bytes: BYTES };
        let inter = CraftFont { family: "Inter", style: "Regular", scripts: LATN, bytes: BYTES };
        let faces = [biz_bold, biz, hans, inter];
        // Chinese mode: the Hans group first (manifest order), then Japanese.
        let zh: Vec<&str> = order_cjk([&faces[0], &faces[2], &faces[1]], true).iter().map(|f| f.family).collect();
        assert_eq!(zh, ["FakeHans", "BIZ UDPGothic", "BIZ UDPGothic"]);
        // Japanese mode keeps the historical default: the Japanese UI face first.
        let ja: Vec<&str> = order_cjk([&faces[0], &faces[2], &faces[1]], false).iter().map(|f| f.family).collect();
        assert_eq!(ja, ["BIZ UDPGothic", "BIZ UDPGothic", "FakeHans"]);
        // Latin-only faces are never interface CJK fallbacks.
        assert!(order_cjk(&faces, true).iter().all(|f| f.covers("Hans") || f.covers("Jpan")));
        // A serif listed first in the manifest never leads the Japanese group, in either mode.
        let mincho = CraftFont { family: "Shippori Mincho", style: "Regular", scripts: JPAN_LATN, bytes: BYTES };
        let zh: Vec<&str> = order_cjk([&mincho, &faces[2], &faces[1]], true).iter().map(|f| f.family).collect();
        assert_eq!(zh, ["FakeHans", "BIZ UDPGothic", "Shippori Mincho"]);
        let ja: Vec<&str> = order_cjk([&mincho, &faces[2], &faces[1]], false).iter().map(|f| f.family).collect();
        assert_eq!(ja, ["BIZ UDPGothic", "Shippori Mincho", "FakeHans"]);
    }

    #[test]
    fn document_faces_match_style_without_inventing_missing_weights() {
        let faces = [
            CraftFont { family: "Shippori Mincho", style: "Regular", scripts: &["Jpan"], bytes: b"serif" },
            CraftFont { family: "BIZ UDPGothic", style: "Regular", scripts: &["Jpan"], bytes: b"sans" },
            CraftFont { family: "BIZ UDPGothic", style: "Bold", scripts: &["Jpan"], bytes: b"bold" },
            CraftFont { family: "BIZ UDPGothic", style: "Bold", scripts: &["Latn"], bytes: b"not-japanese" },
        ];
        assert_eq!(document_face(&faces, false, false).unwrap().bytes, b"sans");
        assert_eq!(document_face(&faces, false, true).unwrap().bytes, b"bold");
        assert_eq!(document_face(&faces, true, false).unwrap().bytes, b"serif");
        assert_eq!(document_face(&faces, true, true).unwrap().bytes, b"serif");
        assert_eq!(document_face(&faces[..2], false, true).unwrap().bytes, b"sans");
        assert_eq!(document_face(&faces[..1], false, true).unwrap().bytes, b"serif");
        // Every Japanese face is a candidate, the style's match first and none twice.
        let order: Vec<&[u8]> = document_faces(&faces, false, true).iter().map(|f| f.bytes).collect();
        assert_eq!(order, [b"bold".as_slice(), b"sans", b"serif"]);
        let order: Vec<&[u8]> = document_faces(&faces, true, false).iter().map(|f| f.bytes).collect();
        assert_eq!(order, [b"serif".as_slice(), b"sans", b"bold"]);
        assert!(document_face(&faces[3..], false, true).is_none());
        assert!(document_face(&[], false, true).is_none());
    }

    #[test]
    fn arabic_faces_are_picked_by_script_in_manifest_order() {
        // Synthetic faces: the filter must not depend on the real build input.
        static BYTES: &[u8] = b"fake";
        static ARAB_LATN: &[&str] = &["Arab", "Latn"];
        static JPAN: &[&str] = &["Jpan"];
        let naskh = CraftFont { family: "FakeNaskh", style: "Regular", scripts: ARAB_LATN, bytes: BYTES };
        let biz = CraftFont { family: "BIZ UDPGothic", style: "Regular", scripts: JPAN, bytes: BYTES };
        let kufi = CraftFont { family: "FakeKufi", style: "Regular", scripts: ARAB_LATN, bytes: BYTES };
        let faces = [naskh, biz, kufi];
        let ar: Vec<&str> = arabic(&faces).iter().map(|f| f.family).collect();
        assert_eq!(ar, ["FakeNaskh", "FakeKufi"]);
        // An Arabic face is never a CJK fallback, and the other way round.
        assert!(order_cjk(&faces, false).iter().all(|f| f.family == "BIZ UDPGothic"));
        assert_eq!(ui_arabic_fonts().len(), CRAFT_FONTS.iter().filter(|f| f.covers("Arab")).count());
    }

    #[test]
    fn hebrew_document_faces_follow_the_weight() {
        static HEBR_LATN: &[&str] = &["Hebr", "Latn"];
        let faces = [
            CraftFont { family: "BIZ UDPGothic", style: "Bold", scripts: &["Jpan"], bytes: b"not-hebrew" },
            CraftFont { family: "FakeHebrew", style: "Regular", scripts: HEBR_LATN, bytes: b"regular" },
            CraftFont { family: "FakeHebrew", style: "Bold", scripts: HEBR_LATN, bytes: b"bold" },
        ];
        assert_eq!(hebrew_face(&faces, false).map(|f| f.bytes), Some(&b"regular"[..]));
        assert_eq!(hebrew_face(&faces, true).map(|f| f.bytes), Some(&b"bold"[..]));
        // No bold face: Regular, never a face of another script.
        assert_eq!(hebrew_face(&faces[..2], true).map(|f| f.bytes), Some(&b"regular"[..]));
        assert!(hebrew_face(&faces[..1], false).is_none());
        assert_eq!(ui_hebrew_fonts().len(), CRAFT_FONTS.iter().filter(|f| f.covers("Hebr")).count());
        assert!(document_hebrew_font(false).is_some());
        // A craft-fonts Hebrew face, when present, is preferred over the bundled fallback.
        if CRAFT_FONTS.iter().any(|f| f.covers("Hebr")) {
            let font = document_hebrew_font(false).unwrap();
            assert!(CRAFT_FONTS.iter().any(|f| std::ptr::eq(f, font)));
        }
    }

    #[test]
    fn hebrew_fallback_is_bundled_without_craft_fonts() {
        // A Hebrew face must be available even when built without `CRAFT_FONTS_DIR`,
        // from a bundled fallback rather than from `CRAFT_FONTS` alone.
        let font = document_hebrew_font(false);
        assert!(font.is_some(), "no Hebrew face without craft-fonts");
        let font = font.unwrap();
        assert!(font.covers("Hebr"));
        assert!(!font.bytes.is_empty());
        if !CRAFT_FONTS.iter().any(|f| f.covers("Hebr")) {
            assert!(std::ptr::eq(font, &HEBREW_FALLBACK), "expected the bundled fallback without craft-fonts");
        }
    }

    /// `build.rs` embeds the Japanese CJK faces whole and any other CJK face only up to
    /// `MAX_OTHER_CJK_BYTES` (the explicitly pinned native-only LXGW UI fallback has a separate
    /// 16 MiB cap), so a checkout with a large Chinese or Korean face (craft-fonts has the
    /// 16.4 MB Noto Sans CJK SC) leaves it out. The four Japanese faces of the pinned craft-fonts
    /// are 24.1 MB.
    #[test]
    fn cjk_faces_follow_the_size_rule() {
        use crate::cjk_scripts::{MAX_NATIVE_UI_CJK_BYTES, MAX_OTHER_CJK_BYTES, NATIVE_UI_CJK_FAMILIES, WHOLE_CJK_FAMILIES, is_cjk_script};
        // A tag can carry a region after the code: craft-fonts tags Noto Sans HK UI `Hant-HK`.
        assert!(["Hant-HK", "Hans", "Jpan", "Hang"].iter().all(|s| is_cjk_script(s)));
        assert!(!["Latn", "Arab", "Hebr"].iter().any(|s| is_cjk_script(s)));
        let cjk = CRAFT_FONTS.iter().filter(|f| f.scripts.iter().any(|s| is_cjk_script(s)));
        let (whole, other): (Vec<_>, Vec<_>) = cjk.partition(|f| WHOLE_CJK_FAMILIES.contains(&f.family));
        for face in &other {
            let len = face.bytes.len();
            let native_ui = NATIVE_UI_CJK_FAMILIES.contains(&face.family);
            assert!(!cfg!(target_arch = "wasm32") || !native_ui, "native UI face in web payload");
            let cap = if native_ui { MAX_NATIVE_UI_CJK_BYTES } else { MAX_OTHER_CJK_BYTES };
            assert!(u64::try_from(len).is_ok_and(|n| n <= cap), "{} is embedded at {len} bytes", face.name());
        }
        let bytes: usize = whole.iter().map(|f| f.bytes.len()).sum();
        assert!(bytes < 25_000_000, "{bytes} bytes of Japanese faces");
    }

    /// The bundled Inter faces carry the Latin, Greek and Cyrillic letters the craft-fonts faces
    /// lack: Uzbek қ ғ ҳ, Kazakh ә ө ү ұ һ, Ukrainian ї є ґ, Greek. This one runs with or without
    /// CRAFT_FONTS_DIR (#2593).
    #[test]
    fn latin_faces_carry_the_letters_the_japanese_faces_lack() {
        for face in document_latin_fonts(false).into_iter().chain(document_latin_fonts(true)) {
            for ch in ['қ', 'ғ', 'ҳ', 'ә', 'ө', 'ү', 'ұ', 'һ', 'ї', 'є', 'ґ', 'ё', 'α', 'β', 'γ', '≥', '≈'] {
                assert!(crate::japanese_glyph_from(face, ch).is_ok(), "{} has no {ch}", face.name());
            }
        }
        assert_eq!(document_latin_fonts(false)[0].style, "Regular");
        assert_eq!(document_latin_fonts(true)[0].style, "SemiBold");
    }

    #[test]
    fn craft_fonts_are_optional_and_consistent() {
        // Holds with or without CRAFT_FONTS_DIR.
        assert_eq!(SHIPPORI_MINCHO.is_some(), CRAFT_FONTS.iter().any(|f| f.family == "Shippori Mincho" && f.style == "Regular"));
        assert_eq!(document_japanese_font().is_some(), CRAFT_FONTS.iter().any(|f| f.covers("Jpan")));
        let ui = ui_japanese_fonts();
        assert_eq!(ui.len(), CRAFT_FONTS.iter().filter(|f| f.covers("Jpan")).count());
        let ui_zh = ui_chinese_fonts();
        assert_eq!(ui_zh.len(), CRAFT_FONTS.iter().filter(|f| f.covers("Hans")).count());
        if CRAFT_FONTS.is_empty() {
            eprintln!("built without craft-fonts (CRAFT_FONTS_DIR unset): no Japanese faces, as expected");
            assert!(SHIPPORI_MINCHO.is_none() && ui.is_empty() && ui_zh.is_empty());
            return;
        }
        assert!(CRAFT_FONTS.iter().all(|f| !f.bytes.is_empty()));
        if CRAFT_FONTS.iter().any(|f| f.family == "BIZ UDPGothic") {
            assert_eq!(ui.first().map(|f| f.family), Some("BIZ UDPGothic"));
        }
        assert_eq!(document_japanese_font().map(|f| f.family), Some("Shippori Mincho"));
    }

    #[test]
    fn hebrew_fallback_glyphs_differ_per_letter() {
        let alef = crate::japanese_glyph_from(&HEBREW_FALLBACK, 'א').unwrap();
        let bet = crate::japanese_glyph_from(&HEBREW_FALLBACK, 'ב').unwrap();
        let gimel = crate::japanese_glyph_from(&HEBREW_FALLBACK, 'ג').unwrap();
        assert!(!alef.contours.is_empty());
        assert_ne!(alef, bet, "distinct Hebrew letters must not share the same outline");
        assert_ne!(alef, gimel, "distinct Hebrew letters must not share the same outline");
        assert_ne!(bet, gimel, "distinct Hebrew letters must not share the same outline");
    }

    #[test]
    fn hebrew_fallback_letters_have_non_rectangular_outlines() {
        // Representative letters with their expected contour counts in the bundled font.
        let expected: [(char, usize); 8] = [('א', 3), ('ב', 2), ('ג', 3), ('ד', 3), ('ה', 3), ('ח', 3), ('ל', 1), ('ו', 1)];
        for (ch, contours) in expected {
            let glyph = crate::japanese_glyph_from(&HEBREW_FALLBACK, ch).unwrap();
            assert_eq!(glyph.contours.len(), contours, "{ch} should have {contours} contours");
            assert!(glyph.width > 0.0);
            // Not a single rectangle: either several contours, or a contour that is not
            // a 4-point axis-aligned rectangle.
            let is_single_rectangle = glyph.contours.len() == 1 && glyph.contours[0].len() == 4 && {
                let c = &glyph.contours[0];
                c[0][0] == c[3][0] && c[1][0] == c[2][0] && c[0][1] == c[1][1] && c[2][1] == c[3][1]
            };
            assert!(!is_single_rectangle, "{ch} must not be a single rectangle");
        }
        // Vav and shin are single contours but with more than 4 points.
        for ch in ['ו', 'ש'] {
            let glyph = crate::japanese_glyph_from(&HEBREW_FALLBACK, ch).unwrap();
            assert!(glyph.contours.iter().any(|c| c.len() > 4), "{ch} should have a non-rectangular contour");
        }
        // Every Hebrew letter in the block has a non-empty, non-rectangular outline.
        for ch in ('\u{05D0}'..='\u{05EA}') {
            let glyph = crate::japanese_glyph_from(&HEBREW_FALLBACK, ch).unwrap();
            assert!(!glyph.contours.is_empty(), "{ch} has no outline");
            let is_single_rectangle = glyph.contours.len() == 1 && glyph.contours[0].len() == 4;
            assert!(!is_single_rectangle, "{ch} must not be a single rectangle");
        }
        // Space stays empty, digits are non-rectangular too.
        let space = crate::japanese_glyph_from(&HEBREW_FALLBACK, ' ').unwrap();
        assert!(space.contours.is_empty());
        let digit = crate::japanese_glyph_from(&HEBREW_FALLBACK, '5').unwrap();
        assert!(!digit.contours.is_empty() && !(digit.contours.len() == 1 && digit.contours[0].len() == 4));
    }

    #[test]
    fn hebrew_fallback_covers_every_letter_and_digit_distinctly() {
        // Parse the bundled font through the same skrifa path callers use and check
        // every Hebrew letter (including final forms) and digit, not just a sample.
        let mut chars: Vec<char> = ('\u{05D0}'..='\u{05EA}').collect();
        chars.extend('0'..='9');
        let mut outlines = Vec::new();
        for ch in &chars {
            let glyph = crate::japanese_glyph_from(&HEBREW_FALLBACK, *ch)
                .unwrap_or_else(|_| panic!("{ch} (U+{:04X}) has no outline in the bundled Hebrew font", *ch as u32));
            assert!(!glyph.contours.is_empty(), "{ch} has no outline");
            assert!(glyph.width > 0.0, "{ch} has no advance");
            assert!(glyph.bbox[2] > glyph.bbox[0] && glyph.bbox[3] > glyph.bbox[1], "{ch} has an empty bounding box");
            let is_single_rectangle = glyph.contours.len() == 1 && glyph.contours[0].len() == 4;
            assert!(!is_single_rectangle, "{ch} must not be a single rectangle");
            // No house-shaped 5-point outline remains for any letter or digit.
            assert!(!glyph.contours.iter().any(|c| c.len() == 5), "{ch} still uses the house-shaped outline");
            for (other, other_glyph) in chars.iter().zip(&outlines) {
                assert_ne!(&glyph, other_glyph, "{ch} and {other} must not share the same outline");
            }
            outlines.push(glyph);
        }
        // Representative Hebrew text parses and every cluster glyph is non-empty.
        for ch in "שלום עולם 123".chars().filter(|c| !c.is_whitespace()) {
            let glyph = crate::japanese_glyph_from(&HEBREW_FALLBACK, ch).unwrap();
            assert!(!glyph.contours.is_empty(), "{ch} in representative text has no outline");
        }
        // Space behavior is preserved: mapped, empty outline, positive advance.
        let space = crate::japanese_glyph_from(&HEBREW_FALLBACK, ' ').unwrap();
        assert!(space.contours.is_empty() && space.width > 0.0);
    }
}
