use super::blocks::{BlockStyle, TextBlock};
use super::fallback::MAX_TYPE3_GLYPHS;
use super::lines::{content_streams, page_dict, rewritten_contents, splice, text_lines};
use super::rewrite::rewrite_block_from;
use super::{DecodeBudget, Document, EditError, LineEdit, Metrics, Object, TextLine, TextSession};
use pdfcraft_content::Pieces;

/// An embedded sfnt source face, read only for the inline editor. Bytes stay in memory and
/// are never installed, copied into assets, or used as a font for another document.
#[derive(Clone, Debug)]
pub struct TextEditorFont {
    pub resource: String,
    pub face: String,
    pub bytes: Vec<u8>,
}

pub fn text_editor_fonts(doc: &Document, page: usize) -> Result<Vec<TextEditorFont>, EditError> {
    let p = page_dict(doc, page)?;
    let resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let mut budget = 32_usize << 20;
    // Every program decoded counts, usable or not, so aliases of one program can't repeat the work.
    let mut decode = DecodeBudget::new(2 * budget, 0);
    let mut result = Vec::new();
    for (name, font) in fonts.iter().take(128) {
        let Some(font) = doc.dict(font) else { continue };
        let Some(bytes) = pdfcraft_fonts::pdf::embedded_sfnt_within(doc, &font, &mut decode) else { continue };
        if bytes.len() > budget {
            continue;
        }
        budget -= bytes.len();
        result.push(TextEditorFont {
            resource: String::from_utf8_lossy(name).into_owned(),
            face: Metrics::from_dict(doc, &font).base_font.rsplit('+').next().unwrap_or_default().to_owned(),
            bytes,
        });
    }
    // The approved full face follows all source subsets in the editor's font stack. It can
    // display newly inserted glyphs before Apply without changing the source text's face.
    let source_faces: Vec<_> = result.iter().map(|p| p.face.clone()).collect();
    for face in source_faces {
        for full in pdfcraft_fonts::embedding::matching_faces(&face) {
            if full.bytes.len() > budget || result.iter().any(|p| p.resource == format!("PCApproved{}", full.name())) {
                continue;
            }
            budget -= full.bytes.len();
            result.push(TextEditorFont { resource: format!("PCApproved{}", full.name()), face: face.clone(), bytes: full.bytes.to_vec() });
        }
    }
    Ok(result)
}

/// Load an explicit full face into this page's resources. Nothing on the page is replaced;
/// the new resource can complete matching source subsets or be chosen in a text style.
pub fn register_text_font(doc: &mut Document, page: usize, bytes: &[u8]) -> Result<String, EditError> {
    pdfcraft_fonts::embedding::validate_document_input(bytes).map_err(EditError::Invalid)?;
    let (_, postscript) = pdfcraft_fonts::embedding::names(bytes).map_err(EditError::Invalid)?;
    let p = page_dict(doc, page)?;
    let mut resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let mut fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let name = pdfcraft_fonts::embedding::embed(doc, &mut fonts, bytes, &postscript).map_err(EditError::Invalid)?;
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    doc.update_dict(p.obj, |d| d.set(b"Resources".to_vec(), Object::Dict(resources)))?;
    Ok(name)
}

/// Read-only draft bounds, measured with the same PDF rewrite used by Apply/Save.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditLayout {
    pub source_rect: [f64; 4],
    pub draft_rect: Option<[f64; 4]>,
    pub overflow: bool,
}

pub fn text_edit_layout(doc: &Document, page: usize, block: usize, text: &str, style: &BlockStyle) -> Result<TextEditLayout, EditError> {
    TextSession::open(doc, page, block)?.layout(text, style)
}

/// The live document's rewrite behind an edit: it strips the paragraph's clip, so the same
/// clip-free layout `layout_draft` measured is what Apply saves.
pub fn apply_text_edit_layout(doc: &mut Document, page: usize, block: usize, text: &str, style: &BlockStyle) -> Result<LineEdit, EditError> {
    // A session opened on the stripped document rewrites with the stripped
    // operator indices the clip removal leaves behind.
    let session = TextSession::open(doc, page, block)?;
    strip_clip_for_block(doc, page, session.source())?;
    let session = TextSession::open(doc, page, block)?;
    session.commit(doc, Some(text), style)
}

/// A draft of paragraph `source` (number `block` among the page's `lines`) laid out by the rewrite
/// Apply runs, on a copy of `doc`.
pub(crate) fn layout_draft(
    doc: &Document,
    page: usize,
    lines: &[TextLine],
    source: &TextBlock,
    block: usize,
    text: &str,
    style: &BlockStyle,
) -> Result<TextEditLayout, EditError> {
    if text == source.editor_text && *style == BlockStyle::default() {
        return Ok(TextEditLayout { source_rect: source.rect, draft_rect: Some(source.rect), overflow: false });
    }
    // A paragraph drawn inside a clipping path keeps that clip in the saved rewrite, so rows
    // grown past it would be clipped away. The editor's draft preview must already draw
    // those rows, and what Apply saves must show them too: strip the clip from both the
    // measured draft and the live document before the rewrite, so draft bounds and the
    // saved PDF agree.
    let mut draft_doc = doc.clone();
    strip_clip_for_block(&mut draft_doc, page, source)?;
    let source_rect = source.rect;
    // Subtract the untouched runs by value, rather than by operator offsets: a rewrite may
    // split BT/ET or add font resources. A multiset also handles coincident repeated text.
    let key =
        |line: &TextLine| (line.text.clone(), line.font.clone(), line.rect.map(f64::to_bits), line.size.to_bits(), line.color.map(f64::to_bits));
    let mut untouched = std::collections::BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        if !source.lines.contains(&index) {
            *untouched.entry(key(line)).or_insert(0_usize) += 1;
        }
    }
    // The draft is measured from the clone whose clip was stripped, so its bounds are the
    // bounds the saved PDF will have. `rewrite_block_from` re-reads the page's lines itself,
    // so it needs lines from the stripped document, whose operator indices are shifted
    // past the removed clip operators.
    let lines = text_lines(&draft_doc, page)?;
    rewrite_block_from(&mut draft_doc, page, lines, block, Some(text), style, MAX_TYPE3_GLYPHS)?;
    let draft = draft_doc;
    let mut rect: Option<[f64; 4]> = None;
    for line in text_lines(&draft, page)? {
        if let Some(count) = untouched.get_mut(&key(&line))
            && *count > 0
        {
            *count -= 1;
            continue;
        }
        // Spaces typed after a row's last word are drawn but show nothing, so an upright row is
        // measured to the end of its last word: they don't make a draft overflow (#4199).
        let mut r = line.rect;
        if line.upright() && !line.ink.is_empty() {
            if crate::bidi::rtl_base(&line.text) {
                r[0] = r[0].max(line.ink.iter().map(|i| i[0]).fold(f64::INFINITY, f64::min));
            } else {
                r[2] = r[2].min(line.ink.iter().map(|i| i[2]).fold(f64::NEG_INFINITY, f64::max));
            }
        }
        rect = Some(rect.map_or(r, |a| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])]));
    }
    let overflow = rect
        .is_some_and(|r| r[0] < source_rect[0] - 0.5 || r[1] < source_rect[1] - 0.5 || r[2] > source_rect[2] + 0.5 || r[3] > source_rect[3] + 0.5);
    Ok(TextEditLayout { source_rect, draft_rect: rect, overflow })
}

/// Remove the rectangle clip in force at the paragraph from `doc`, so the rewrite can draw
/// past it. Only a single `re W n`-style clip is removed, and anything more complex is left.
/// This edits `doc` in place because both the measured draft and the Apply/Commit path go
/// through the same rewrite; stripping the clip here makes rows grown past it visible in
/// the editor and in the saved PDF.
fn strip_clip_for_block(doc: &mut Document, page: usize, source: &TextBlock) -> Result<(), EditError> {
    let Some(&first) = source.lines.first() else { return Ok(()) };
    let lines = text_lines(doc, page)?;
    let Some(target) = lines.get(first) else { return Ok(()) };
    if target.ops.is_empty() { return Ok(()) }
    let at = target.ops.iter().min().copied().unwrap_or(0);
    let p = page_dict(doc, page)?;
    let streams = content_streams(doc, &p.dict)?;
    let pieces = Pieces::join(&streams.iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>());
    let ops = pieces.parse().ops;
    // The rectangle clip we are removing, plus the W and its terminating paint/nop:
    // removing the W alone leaves the path to be painted. `pending` collects the
    // path-construction ops since the last paint; when a paint follows a W, the whole
    // group (path, W and that paint) is the clip in force at the paragraph. Unrelated
    // ops between the path and the W are kept; they do not clear the pending path.
    let mut clip_ops: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut pending: Vec<usize> = Vec::new();
    let mut pending_w: Option<usize> = None;
    for (i, op) in ops.iter().enumerate().take(at) {
        match op.op.as_slice() {
            b"W" | b"W*" => {
                pending_w = Some(i);
            }
            b"n" | b"f" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"S" | b"s" | b"sh" => {
                if let Some(w) = pending_w.take() {
                    // Only the path ops at or after the W's operands belong to this clip;
                    // the W itself sits between them and the paint.
                    clip_ops.insert(w);
                    clip_ops.insert(i);
                    clip_ops.extend(pending.iter().copied());
                }
                pending.clear();
            }
            b"re" | b"m" | b"l" | b"c" | b"v" | b"y" | b"h" => pending.push(i),
            _ => {}
        }
    }
    if clip_ops.is_empty() { return Ok(()) }
    // The rewrite expects op indices relative to `ops`; we reuse `splice` with the same
    // operator numbering `rewrite_block_from` uses, which is also the joined-stream numbering.
    // Removing keeps the graphics state stack balanced (`Q` still matches its `q`).
    let new_data = splice(&pieces, &ops, |i| (Vec::new(), !clip_ops.contains(&i)));
    let contents = rewritten_contents(doc, &streams, new_data);
    let page_ref = p.obj;
    doc.update_dict(page_ref, |d| d.set(b"Contents".to_vec(), contents))?;
    Ok(())
}
