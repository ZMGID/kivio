//! Export Study marks as standard, editable PDF annotations with explicit
//! appearances. Coordinates are normalized to PDF.js's rotated, cropped viewport.
use super::{operations::PdfKind, ReadingNote, Rect};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::{collections::HashSet, io::Write, path::Path};

type Point = [f32; 2];

pub(super) fn write_pdf(
    source: &Path,
    target: &mut impl Write,
    notes: &[ReadingNote],
    kind: PdfKind,
    run: Option<&str>,
) -> Result<(), String> {
    let marks: Vec<_> = notes
        .iter()
        .filter(|note| {
            note.kind == "highlight"
                && note
                    .anchor
                    .as_ref()
                    .map_or(kind == PdfKind::Source, |anchor| match &anchor.view {
                        None => kind == PdfKind::Source,
                        Some(view) => view.kind == kind && Some(view.run_id.as_str()) == run,
                    })
        })
        .collect();
    if marks.is_empty() {
        std::io::copy(
            &mut std::fs::File::open(source).map_err(|e| e.to_string())?,
            target,
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut pdf = Document::load(source).map_err(|e| format!("无法读取 PDF 批注结构：{e}"))?;
    if pdf.is_encrypted() {
        return Err("此 PDF 已加密，无法写入标注。请先提供未加密的副本。".into());
    }
    let pages = pdf.get_pages();
    for note in marks {
        let anchor = note
            .anchor
            .as_ref()
            .ok_or("有缺少页面位置的旧标注，无法完整导出")?;
        let page_number = u32::try_from(anchor.page).map_err(|_| "标注页码无效")?;
        let page = *pages
            .get(&page_number)
            .ok_or_else(|| format!("标注引用的第 {} 页不存在", anchor.page))?;
        let geometry = PageGeometry::read(&pdf, page)?;
        let rects = if anchor.rects.is_empty() {
            vec![anchor
                .rect
                .as_ref()
                .ok_or_else(|| format!("第 {} 页有缺少位置的旧标注，无法完整导出", anchor.page))?]
        } else {
            anchor.rects.iter().collect()
        };
        let mut quads = Vec::with_capacity(rects.len());
        for rect in rects {
            let quad = geometry.quad(rect)?;
            if !quads.contains(&quad) {
                quads.push(quad);
            }
        }
        let color = color(anchor.color.as_deref().unwrap_or("#d3a51d"))?;
        let mark = anchor.mark.as_deref().unwrap_or("highlight");
        let annotation = annotation(&mut pdf, page, &note.id, &note.content, mark, color, &quads)?;
        // Annots may itself be indirect; preserve all existing entries (including
        // links and widgets) without mutating an array shared with another page.
        let mut annots = match pdf
            .get_dictionary(page)
            .map_err(|e| e.to_string())?
            .get(b"Annots")
        {
            Ok(value) => match pdf.dereference(value).map_err(|e| e.to_string())?.1 {
                Object::Null => vec![],
                value => value
                    .as_array()
                    .map_err(|e| format!("无法读取原有 PDF 批注：{e}"))?
                    .clone(),
            },
            Err(lopdf::Error::DictKey(_)) => vec![],
            Err(e) => return Err(e.to_string()),
        };
        annots.push(annotation.into());
        pdf.get_dictionary_mut(page)
            .map_err(|e| e.to_string())?
            .set("Annots", annots);
    }
    pdf.save_to(target)
        .map_err(|e| format!("无法写入 PDF 标注：{e}"))
}

struct PageGeometry {
    bounds: [f32; 4],
    rotation: i64,
}
impl PageGeometry {
    fn read(pdf: &Document, page: ObjectId) -> Result<Self, String> {
        fn inherited(
            pdf: &Document,
            mut page: ObjectId,
            key: &[u8],
        ) -> Result<Option<Object>, String> {
            let mut visited = HashSet::new();
            loop {
                if !visited.insert(page) {
                    return Err("PDF 页面继承关系存在循环".into());
                }
                let dict = pdf.get_dictionary(page).map_err(|e| e.to_string())?;
                if let Ok(value) = dict.get(key) {
                    return Ok(Some(
                        pdf.dereference(value).map_err(|e| e.to_string())?.1.clone(),
                    ));
                }
                match dict.get(b"Parent") {
                    Ok(parent) => page = parent.as_reference().map_err(|e| e.to_string())?,
                    Err(_) => return Ok(None),
                }
            }
        }
        fn bounds(value: Object) -> Result<[f32; 4], String> {
            let values = value.as_array().map_err(|e| e.to_string())?;
            if values.len() != 4 {
                return Err("PDF 页面尺寸无效".into());
            }
            let mut result = [0.; 4];
            for (index, value) in values.iter().enumerate() {
                result[index] = value.as_float().map_err(|e| e.to_string())?;
                if !result[index].is_finite() {
                    return Err("PDF 页面尺寸无效".into());
                }
            }
            Ok([
                result[0].min(result[2]),
                result[1].min(result[3]),
                result[0].max(result[2]),
                result[1].max(result[3]),
            ])
        }
        let media = bounds(inherited(pdf, page, b"MediaBox")?.ok_or("PDF 缺少页面尺寸")?)?;
        let mut view = media;
        if let Some(crop) = inherited(pdf, page, b"CropBox")? {
            let crop = bounds(crop)?;
            let intersection = [
                media[0].max(crop[0]),
                media[1].max(crop[1]),
                media[2].min(crop[2]),
                media[3].min(crop[3]),
            ];
            if intersection[2] > intersection[0] && intersection[3] > intersection[1] {
                view = intersection;
            }
        }
        if view[2] <= view[0] || view[3] <= view[1] {
            return Err("PDF 页面尺寸为空".into());
        }
        let rotation = inherited(pdf, page, b"Rotate")?
            .map(|v| v.as_i64())
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or(0)
            .rem_euclid(360);
        // PDF.js treats invalid non-right-angle rotations as zero.
        Ok(Self {
            bounds: view,
            rotation: if rotation % 90 == 0 { rotation } else { 0 },
        })
    }
    fn point(&self, u: f32, v: f32) -> Point {
        let [x0, y0, x1, y1] = self.bounds;
        let (w, h) = (x1 - x0, y1 - y0);
        match self.rotation {
            90 => [x0 + v * w, y0 + u * h],
            180 => [x1 - u * w, y0 + v * h],
            270 => [x1 - v * w, y1 - u * h],
            _ => [x0 + u * w, y1 - v * h],
        }
    }
    fn quad(&self, rect: &Rect) -> Result<[Point; 4], String> {
        if ![rect.x, rect.y, rect.w, rect.h]
            .iter()
            .all(|n| n.is_finite())
            || rect.w <= 0.
            || rect.h <= 0.
        {
            return Err("标注坐标无效，无法导出 PDF".into());
        }
        let (x0, y0) = (rect.x.max(0.) as f32, rect.y.max(0.) as f32);
        let (x1, y1) = (
            (rect.x + rect.w).min(1.) as f32,
            (rect.y + rect.h).min(1.) as f32,
        );
        if x1 <= x0 || y1 <= y0 {
            return Err("标注位于页面范围之外，无法导出 PDF".into());
        }
        Ok([
            self.point(x0, y0),
            self.point(x1, y0),
            self.point(x0, y1),
            self.point(x1, y1),
        ])
    }
}
fn color(value: &str) -> Result<[f32; 3], String> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err("标注颜色无效".into());
    }
    let mut rgb = [0.; 3];
    for (i, component) in rgb.iter_mut().enumerate() {
        *component = u8::from_str_radix(&value[1 + i * 2..3 + i * 2], 16)
            .map_err(|e| e.to_string())? as f32
            / 255.;
    }
    Ok(rgb)
}
fn numbers(values: impl IntoIterator<Item = f32>) -> Object {
    Object::Array(values.into_iter().map(Object::Real).collect())
}
fn text(value: &str) -> Object {
    let bytes = [0xfe, 0xff]
        .into_iter()
        .chain(value.encode_utf16().flat_map(u16::to_be_bytes))
        .collect();
    Object::String(bytes, StringFormat::Hexadecimal)
}
fn annotation(
    pdf: &mut Document,
    page: ObjectId,
    id: &str,
    content: &str,
    mark: &str,
    rgb: [f32; 3],
    quads: &[[Point; 4]],
) -> Result<ObjectId, String> {
    let subtype = match mark {
        "highlight" => "Highlight",
        "underline" => "Underline",
        "strike" => "StrikeOut",
        "area" => "Square",
        _ => return Err("不支持的标注类型".into()),
    };
    let points: Vec<_> = quads.iter().flatten().collect();
    let x0 = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let y0 = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let x1 = points
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let y1 = points
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let opacity = if mark == "highlight" { 0.24 } else { 1. };
    let mut appearance = format!(
        "q /GS gs {} {} {} rg {} {} {} RG 1 w\n",
        rgb[0], rgb[1], rgb[2], rgb[0], rgb[1], rgb[2]
    );
    for quad in quads {
        let q = quad.map(|p| [p[0] - x0, p[1] - y0]);
        if mark == "underline" || mark == "strike" {
            let fraction = if mark == "strike" { 0.5 } else { 0.92 };
            let a = [
                q[0][0] + (q[2][0] - q[0][0]) * fraction,
                q[0][1] + (q[2][1] - q[0][1]) * fraction,
            ];
            let b = [
                q[1][0] + (q[3][0] - q[1][0]) * fraction,
                q[1][1] + (q[3][1] - q[1][1]) * fraction,
            ];
            appearance.push_str(&format!("{} {} m {} {} l S\n", a[0], a[1], b[0], b[1]));
        } else {
            appearance.push_str(&format!(
                "{} {} m {} {} l {} {} l {} {} l h {}\n",
                q[0][0],
                q[0][1],
                q[1][0],
                q[1][1],
                q[3][0],
                q[3][1],
                q[2][0],
                q[2][1],
                if mark == "area" { "S" } else { "f" }
            ));
        }
    }
    appearance.push_str("Q\n");
    let ap = pdf.add_object(Stream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
        "BBox" => numbers([0., 0., x1 - x0, y1 - y0]),
        "Resources" => dictionary! { "ExtGState" => dictionary! { "GS" => dictionary! { "Type" => "ExtGState", "ca" => opacity, "CA" => opacity, "BM" => "Multiply" } } },
    }, appearance.into_bytes()));
    let mut dict: Dictionary = dictionary! {
        "Type" => "Annot", "Subtype" => subtype, "P" => page,
        "Rect" => numbers([x0, y0, x1, y1]), "C" => numbers(rgb), "CA" => opacity,
        "F" => 4, "T" => text("Kivio Study"), "Contents" => text(content), "NM" => text(&format!("kivio-{id}")),
        "AP" => dictionary! { "N" => ap },
    };
    if mark == "area" {
        dict.set("BS", dictionary! { "W" => 1, "S" => "S" });
    } else {
        dict.set(
            "QuadPoints",
            numbers(quads.iter().flatten().flatten().copied()),
        );
    }
    Ok(pdf.add_object(dict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::{Anchor, PdfLocation};
    fn fixture(path: &Path, rotation: i64) {
        let mut doc = Document::with_version("1.7");
        let parent = doc.new_object_id();
        let existing = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![10.into(), 20.into(), 20.into(), 30.into()], "Contents" => text("Existing comment") });
        let annots = doc.add_object(vec![Object::Reference(existing)]);
        let page = doc
            .add_object(dictionary! { "Type" => "Page", "Parent" => parent, "Annots" => annots });
        doc.objects.insert(parent, dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "CropBox" => vec![10.into(), 20.into(), 210.into(), 120.into()], "Rotate" => rotation,
        }.into());
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => parent });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }
    fn note(mark: &str, view: Option<PdfLocation>) -> ReadingNote {
        ReadingNote {
            id: format!("{mark}-1"),
            kind: "highlight".into(),
            content: "中文批注与 evidence".into(),
            anchor: Some(Anchor {
                page: 1,
                text: "evidence".into(),
                rect: Some(Rect {
                    x: 0.1,
                    y: 0.2,
                    w: 0.3,
                    h: 0.4,
                }),
                rects: vec![],
                view,
                mark: Some(mark.into()),
                color: Some("#4b95c9".into()),
            }),
        }
    }
    #[test]
    fn exports_editable_marks_with_appearances_and_preserves_existing_annotations_and_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.pdf");
        fixture(&source, 0);
        let original = std::fs::read(&source).unwrap();
        let mut notes: Vec<_> = ["highlight", "underline", "strike", "area"]
            .map(|mark| note(mark, None))
            .into();
        notes[0].anchor.as_mut().unwrap().rects = vec![
            Rect {
                x: 0.1,
                y: 0.2,
                w: 0.3,
                h: 0.1,
            },
            Rect {
                x: 0.1,
                y: 0.4,
                w: 0.2,
                h: 0.1,
            },
        ];
        for (kind, run, id) in [
            (PdfKind::Mono, "old", "old-mono"),
            (PdfKind::Mono, "current", "current-mono"),
            (PdfKind::Dual, "current", "current-dual"),
        ] {
            let mut translated = note(
                "highlight",
                Some(PdfLocation {
                    kind,
                    run_id: run.into(),
                }),
            );
            translated.id = id.into();
            notes.push(translated);
        }
        for (kind, run, expected) in [
            (PdfKind::Source, None, 5),
            (PdfKind::Mono, Some("current"), 2),
            (PdfKind::Dual, Some("current"), 2),
        ] {
            let mut output = Vec::new();
            write_pdf(&source, &mut output, &notes, kind, run).unwrap();
            let exported = Document::load_mem(&output).unwrap();
            let page = *exported.get_pages().get(&1).unwrap();
            let annots = exported.get_page_annotations(page).unwrap();
            assert_eq!(annots.len(), expected);
            assert_eq!(
                annots[0].get(b"Contents").unwrap(),
                &text("Existing comment")
            );
            for annot in &annots[1..] {
                assert_eq!(
                    annot.get(b"Contents").unwrap(),
                    &text("中文批注与 evidence")
                );
                assert_eq!(annot.get(b"F").unwrap().as_i64().unwrap(), 4);
                let ap = annot
                    .get(b"AP")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"N")
                    .unwrap()
                    .as_reference()
                    .unwrap();
                let stream = exported.get_object(ap).unwrap().as_stream().unwrap();
                assert!(!stream.content.is_empty());
                assert_eq!(
                    stream.dict.get(b"Subtype").unwrap().as_name().unwrap(),
                    b"Form"
                );
            }
            if kind != PdfKind::Source {
                assert_eq!(
                    annots[1].get(b"NM").unwrap(),
                    &text(if kind == PdfKind::Mono {
                        "kivio-current-mono"
                    } else {
                        "kivio-current-dual"
                    })
                );
            }
            if kind == PdfKind::Source {
                assert_eq!(
                    annots[1]
                        .get(b"QuadPoints")
                        .unwrap()
                        .as_array()
                        .unwrap()
                        .len(),
                    16
                );
                for (annot, subtype) in
                    annots[1..]
                        .iter()
                        .zip(["Highlight", "Underline", "StrikeOut", "Square"])
                {
                    assert_eq!(
                        annot.get(b"Subtype").unwrap().as_name().unwrap(),
                        subtype.as_bytes()
                    );
                }
            }
        }
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
    #[test]
    fn maps_display_coordinates_back_through_inherited_crop_and_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.pdf");
        for (rotation, expected) in [
            (0, [30., 100., 90., 100., 30., 60., 90., 60.]),
            (90, [50., 30., 50., 60., 130., 30., 130., 60.]),
            (180, [190., 40., 130., 40., 190., 80., 130., 80.]),
            (270, [170., 110., 170., 80., 90., 110., 90., 80.]),
        ] {
            fixture(&source, rotation);
            let mut output = Vec::new();
            write_pdf(
                &source,
                &mut output,
                &[note("underline", None)],
                PdfKind::Source,
                None,
            )
            .unwrap();
            let exported = Document::load_mem(&output).unwrap();
            let page = exported.get_pages()[&1];
            let annots = exported.get_page_annotations(page).unwrap();
            let quad = annots[1].get(b"QuadPoints").unwrap().as_array().unwrap();
            for (actual, expected) in quad.iter().zip(expected) {
                assert!(
                    (actual.as_float().unwrap() - expected).abs() < 0.001,
                    "rotation {rotation}"
                );
            }
        }
    }
    #[test]
    fn legacy_marks_export_and_invalid_positions_fail_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.pdf");
        fixture(&source, 0);
        let mut legacy = note("highlight", None);
        legacy.anchor.as_mut().unwrap().mark = None;
        legacy.anchor.as_mut().unwrap().color = None;
        let mut output = Vec::new();
        write_pdf(
            &source,
            &mut output,
            &[legacy.clone()],
            PdfKind::Source,
            None,
        )
        .unwrap();
        let exported = Document::load_mem(&output).unwrap();
        assert_eq!(
            exported
                .get_page_annotations(exported.get_pages()[&1])
                .unwrap()
                .len(),
            2
        );
        for bad in [
            None,
            Some(Rect {
                x: f64::NAN,
                y: 0.,
                w: 0.1,
                h: 0.1,
            }),
            Some(Rect {
                x: 2.,
                y: 0.,
                w: 0.1,
                h: 0.1,
            }),
        ] {
            legacy.anchor.as_mut().unwrap().rect = bad;
            let mut output = Vec::new();
            assert!(write_pdf(
                &source,
                &mut output,
                &[legacy.clone()],
                PdfKind::Source,
                None
            )
            .is_err());
            assert!(output.is_empty());
        }
        legacy.anchor = None;
        assert!(write_pdf(&source, &mut Vec::new(), &[legacy], PdfKind::Source, None).is_err());
        let mut null_annots = Document::load(&source).unwrap();
        let page = null_annots.get_pages()[&1];
        null_annots
            .get_dictionary_mut(page)
            .unwrap()
            .set("Annots", Object::Null);
        null_annots.save(&source).unwrap();
        let mut output = Vec::new();
        write_pdf(
            &source,
            &mut output,
            &[note("area", None)],
            PdfKind::Source,
            None,
        )
        .unwrap();
        let exported = Document::load_mem(&output).unwrap();
        assert_eq!(
            exported
                .get_page_annotations(exported.get_pages()[&1])
                .unwrap()
                .len(),
            1
        );
        let mut output = Vec::new();
        write_pdf(&source, &mut output, &[], PdfKind::Source, None).unwrap();
        assert_eq!(output, std::fs::read(source).unwrap());
    }
}
