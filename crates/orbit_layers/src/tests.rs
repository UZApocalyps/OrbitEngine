//! Fixtures are built here (no binary files): a tiny PSD writer, an ORA and a
//! KRA zip.

use std::io::Write;

use orbit_anim::rig::Rig;

use super::*;

// ---------------------------------------------------------------- PSD

struct L {
    name: &'static str,
    rect: [i32; 4], // top, left, bottom, right
    color: [u8; 3],
    alpha: u8,
    hidden: bool,
    section: u32,
    rle: bool,
    key: Option<&'static [u8; 4]>,
}

impl L {
    fn new(name: &'static str, rect: [i32; 4], color: [u8; 3]) -> Self {
        Self { name, rect, color, alpha: 255, hidden: false, section: 0, rle: false, key: None }
    }
    fn group_start() -> Self {
        Self { section: 3, ..Self::new("</Layer group>", [0; 4], [0; 3]) }
    }
    fn group_end(name: &'static str) -> Self {
        Self { section: 1, ..Self::new(name, [0; 4], [0; 3]) }
    }
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn psd_channel(l: &L, id: i16) -> Vec<u8> {
    let (w, h) = ((l.rect[3] - l.rect[1]).max(0) as usize, (l.rect[2] - l.rect[0]).max(0) as usize);
    if w == 0 || h == 0 {
        return 0u16.to_be_bytes().to_vec();
    }
    let v = match id {
        0..=2 => l.color[id as usize],
        _ => l.alpha,
    };
    let mut out = Vec::new();
    if l.rle {
        out.extend_from_slice(&1u16.to_be_bytes());
        // One run per row: repeat `v` w times (w <= 128).
        for _ in 0..h {
            out.extend_from_slice(&2u16.to_be_bytes());
        }
        for _ in 0..h {
            out.push((1 - w as i32) as i8 as u8);
            out.push(v);
        }
    } else {
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend(std::iter::repeat_n(v, w * h));
    }
    out
}

fn psd(w: u32, h: u32, layers: &[L]) -> Vec<u8> {
    let mut f = b"8BPS".to_vec();
    f.extend_from_slice(&1u16.to_be_bytes());
    f.extend_from_slice(&[0; 6]);
    f.extend_from_slice(&3u16.to_be_bytes());
    be32(&mut f, h);
    be32(&mut f, w);
    f.extend_from_slice(&8u16.to_be_bytes());
    f.extend_from_slice(&3u16.to_be_bytes());
    be32(&mut f, 0);
    be32(&mut f, 0);
    let mut info = Vec::new();
    info.extend_from_slice(&(layers.len() as i16).to_be_bytes());
    let mut data = Vec::new();
    for l in layers {
        for r in l.rect {
            be32(&mut info, r as u32);
        }
        info.extend_from_slice(&4u16.to_be_bytes());
        for id in [0i16, 1, 2, -1] {
            let ch = psd_channel(l, id);
            info.extend_from_slice(&id.to_be_bytes());
            be32(&mut info, ch.len() as u32);
            data.extend(ch);
        }
        info.extend_from_slice(b"8BIMnorm");
        info.extend_from_slice(&[255, 0, if l.hidden { 2 } else { 0 }, 0]);
        let mut extra = Vec::new();
        be32(&mut extra, 0); // mask
        be32(&mut extra, 0); // ranges
        extra.push(l.name.len() as u8);
        extra.extend_from_slice(l.name.as_bytes());
        extra.extend(std::iter::repeat_n(0u8, (4 - (1 + l.name.len()) % 4) % 4));
        if l.section != 0 {
            extra.extend_from_slice(b"8BIMlsct");
            be32(&mut extra, 4);
            be32(&mut extra, l.section);
        }
        if let Some(k) = l.key {
            extra.extend_from_slice(b"8BIM");
            extra.extend_from_slice(k);
            be32(&mut extra, 4);
            be32(&mut extra, 0);
        }
        be32(&mut info, extra.len() as u32);
        info.extend(extra);
    }
    info.extend(data);
    if info.len() % 2 == 1 {
        info.push(0);
    }
    let mut lm = Vec::new();
    be32(&mut lm, info.len() as u32);
    lm.extend(info);
    be32(&mut f, lm.len() as u32);
    f.extend(lm);
    f
}

fn sample() -> Vec<u8> {
    psd(
        100,
        200,
        &[
            L::new("body", [100, 20, 200, 80], [255, 0, 0]),
            L::group_start(),
            L::new("eye", [50, 30, 60, 40], [0, 255, 0]),
            L { hidden: true, rle: true, ..L::new("eye closed", [52, 30, 55, 40], [0, 0, 255]) },
            L::group_end("face"),
            L { key: Some(b"brit"), ..L::new("adjust", [0, 0, 0, 0], [0; 3]) },
        ],
    )
}

#[test]
fn psd_layers_groups_and_visibility() {
    let doc = parse(&sample()).unwrap();
    assert_eq!((doc.width, doc.height), (100, 200));
    assert_eq!(doc.layers.len(), 3);
    assert_eq!(doc.layers[0].name, "body");
    assert_eq!((doc.layers[0].x, doc.layers[0].y, doc.layers[0].width, doc.layers[0].height), (20, 100, 60, 100));
    assert_eq!(&doc.layers[0].rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(doc.groups.len(), 1);
    assert_eq!(doc.groups[0].name, "face");
    assert_eq!(doc.layers[1].groups, vec![0]);
    assert!(doc.layers[0].groups.is_empty());
    assert!(!doc.layers[2].visible);
    assert_eq!(&doc.layers[2].rgba[..4], &[0, 0, 255, 255]);
    assert!(doc.warnings.iter().any(|w| w.contains("adjust")), "{:?}", doc.warnings);
}

#[test]
fn rejects_garbage_and_unsupported() {
    assert!(parse(b"hello").is_err());
    assert!(parse(&sample()[..40]).is_err());
    let mut cmyk = sample();
    cmyk[24..26].copy_from_slice(&4u16.to_be_bytes());
    assert!(parse(&cmyk).unwrap_err().contains("RGB"));
    let mut deep = sample();
    deep[22..24].copy_from_slice(&32u16.to_be_bytes());
    assert!(parse(&deep).is_err());
}

#[test]
fn trim_crops_to_alpha() {
    let mut rgba = vec![0u8; 4 * 4 * 4];
    let at = (4 + 2) * 4; // pixel (2, 1)
    rgba[at..at + 4].copy_from_slice(&[9, 9, 9, 255]);
    let mut l = Layer { name: "t".into(), groups: vec![], visible: true, opacity: 1.0, x: 10, y: 20, width: 4, height: 4, rgba };
    assert!(trim(&mut l));
    assert_eq!((l.x, l.y, l.width, l.height), (12, 21, 1, 1));
    assert_eq!(l.rgba, vec![9, 9, 9, 255]);
    let mut empty = Layer { rgba: vec![0; 16], width: 2, height: 2, ..l };
    assert!(!trim(&mut empty));
}

// ---------------------------------------------------------------- rig

fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn run(doc: &LayeredDoc, opts: &ImportOptions) -> (Rig, ImportReport) {
    let mut rig = Rig::default();
    let mut n = 0;
    let report = import_into_rig(&mut rig, doc, opts, &mut |name, _, _, _| {
        n += 1;
        Ok(format!("layers/{n}_{name}.png"))
    })
    .unwrap();
    (rig, report)
}

#[test]
fn imports_into_a_rig_at_canvas_positions() {
    let doc = parse(&sample()).unwrap();
    let (rig, report) = run(&doc, &ImportOptions::default());
    assert_eq!(report.slots, ["body", "eye", "eye closed"]);
    assert_eq!(report.hidden, ["eye closed"]);
    assert_eq!(rig.bones.len(), 1);
    assert_eq!(rig.slots.len(), 3);
    assert!(rig.slots[2].attachment.is_none());
    assert!(rig.slots[0].attachment.is_some());
    assert!(rig.validate().iter().all(|i| i.severity != orbit_anim::rig::Severity::Error), "{:?}", rig.validate());
    // body: canvas (20..80, 100..200) -> rig x in -30..30, y in -100..0.
    let m = rig.skins[0].attachments.values().find_map(|a| match a {
        orbit_anim::rig::Attachment::Mesh(m) if m.image.contains("body") => Some(m),
        _ => None,
    });
    let m = m.unwrap();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for v in &m.vertices {
        for k in 0..2 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    assert!((lo[0] + 30.0).abs() < 0.6 && (hi[0] - 30.0).abs() < 0.6, "{lo:?} {hi:?}");
    assert!((lo[1] + 100.0).abs() < 0.6 && hi[1].abs() < 0.6, "{lo:?} {hi:?}");
}

#[test]
fn groups_become_bones_and_hidden_can_be_dropped() {
    let doc = parse(&sample()).unwrap();
    let (rig, report) = run(&doc, &ImportOptions { groups_as_bones: true, include_hidden: false });
    assert_eq!(report.slots, ["body", "eye"]);
    assert_eq!(report.bones, ["root", "face"]);
    let face = rig.bones.iter().find(|b| b.name == "face").unwrap();
    let eye = rig.slots.iter().find(|s| s.name == "eye").unwrap();
    assert_eq!(eye.bone, face.id);
    let body = rig.slots.iter().find(|s| s.name == "body").unwrap();
    assert_ne!(body.bone, face.id);
}

#[test]
fn adds_to_an_existing_rig_with_unique_names() {
    let doc = parse(&sample()).unwrap();
    let (mut rig, _) = run(&doc, &ImportOptions::default());
    let mut n = 0;
    import_into_rig(&mut rig, &doc, &ImportOptions::default(), &mut |_, _, _, _| {
        n += 1;
        Ok(format!("again/{n}.png"))
    })
    .unwrap();
    assert_eq!(rig.slots.len(), 6);
    assert_eq!(rig.bones.len(), 1);
    let mut names: Vec<_> = rig.slots.iter().map(|s| s.name.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 6);
}

#[test]
fn an_empty_document_is_an_error() {
    let doc = parse(&psd(10, 10, &[L::new("blank", [0, 0, 5, 5], [0; 3])])).unwrap();
    let mut doc = doc;
    doc.layers[0].rgba.iter_mut().skip(3).step_by(4).for_each(|a| *a = 0);
    let mut rig = Rig::default();
    let err = import_into_rig(&mut rig, &doc, &ImportOptions::default(), &mut |_, _, _, _| Ok("x.png".into())).unwrap_err();
    assert!(err.contains("no raster layer"));
}

// ---------------------------------------------------------------- ORA / KRA

fn zip_of(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in files {
        w.start_file(*name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[test]
fn ora_stack_order_groups_and_visibility() {
    let red = png(2, 2, &[255, 0, 0, 255].repeat(4));
    let blue = png(1, 1, &[0, 0, 255, 255]);
    let xml = r#"<?xml version="1.0"?>
<image w="50" h="60"><stack>
  <stack name="face &amp; hair" visibility="visible">
    <layer name="top" src="data/top.png" x="5" y="6" visibility="hidden" opacity="0.5"/>
  </stack>
  <layer name="bottom" src="data/bottom.png" x="1" y="2" visibility="visible"/>
</stack></image>"#;
    let file = zip_of(&[("mimetype", b"image/openraster".to_vec()), ("stack.xml", xml.as_bytes().to_vec()), ("data/top.png", blue), ("data/bottom.png", red)]);
    let doc = parse(&file).unwrap();
    assert_eq!((doc.width, doc.height), (50, 60));
    assert_eq!(doc.layers[0].name, "bottom");
    assert_eq!(doc.layers[1].name, "top");
    assert_eq!(doc.groups[0].name, "face & hair");
    assert_eq!(doc.layers[1].groups, vec![0]);
    assert!(!doc.layers[1].visible);
    assert_eq!((doc.layers[1].x, doc.layers[1].y), (5, 6));
    assert!((doc.layers[1].opacity - 0.5).abs() < 1e-6);
    let (rig, _) = run(&doc, &ImportOptions::default());
    assert_eq!(rig.slots.len(), 2);
}

fn lzf_literal(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(32) {
        out.push(chunk.len() as u8 - 1);
        out.extend_from_slice(chunk);
    }
    out
}

fn kra_tile(r: u8, g: u8, b: u8, a: u8, lzf: bool) -> Vec<u8> {
    let mut raw = Vec::new();
    for v in [b, g, r, a] {
        raw.extend(std::iter::repeat_n(v, 64 * 64));
    }
    let mut body = vec![u8::from(lzf)];
    body.extend(if lzf { lzf_literal(&raw) } else { raw });
    let mut out = b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 1\n".to_vec();
    out.extend_from_slice(format!("-64,0,LZF,{}\n", body.len()).as_bytes());
    out.extend(body);
    out
}

#[test]
fn kra_tiles_and_group_tree() {
    let xml = r#"<DOC><IMAGE width="128" height="64" name="Doc"><layers>
<layer nodetype="grouplayer" name="g" visible="1" opacity="255"><layers>
  <layer nodetype="paintlayer" name="inner" filename="layer2" visible="0" opacity="255" x="0" y="0" colorspacename="RGBA"/>
</layers></layer>
<layer nodetype="paintlayer" name="base" filename="layer1" visible="1" opacity="255" x="0" y="0" colorspacename="RGBA"/>
<layer nodetype="vectorlayer" name="shapes" visible="1"/>
</layers></IMAGE></DOC>"#;
    let file = zip_of(&[
        ("maindoc.xml", xml.as_bytes().to_vec()),
        ("Doc/layers/layer1", kra_tile(10, 20, 30, 255, true)),
        ("Doc/layers/layer2", kra_tile(1, 2, 3, 128, false)),
    ]);
    let doc = parse(&file).unwrap();
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.layers[0].name, "base");
    assert_eq!(doc.layers[1].name, "inner");
    assert_eq!(doc.layers[1].groups, vec![0]);
    assert!(!doc.layers[1].visible);
    assert_eq!((doc.layers[0].x, doc.layers[0].y, doc.layers[0].width, doc.layers[0].height), (-64, 0, 64, 64));
    assert_eq!(&doc.layers[0].rgba[..4], &[10, 20, 30, 255]);
    assert_eq!(&doc.layers[1].rgba[..4], &[1, 2, 3, 128]);
    assert!(doc.warnings.iter().any(|w| w.contains("shapes")));
}

#[test]
fn lzf_back_references() {
    // "abc" literal, then a back-reference of length 5 at distance 3.
    let packed = [2, b'a', b'b', b'c', 3 << 5, 2];
    assert_eq!(kra::lzf(&packed, 64).unwrap(), b"abcabcab".to_vec());
    assert!(kra::lzf(&[0xE0, 0, 9], 64).is_err());
}

#[test]
fn xml_scanner_handles_attributes_and_entities() {
    let t = xml::tags(r#"<a x="1 &lt; 2" y='q'><!-- <b/> --><c/></a>"#);
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].attr("x"), Some("1 < 2"));
    assert_eq!(t[0].attr("y"), Some("q"));
    assert!(t[1].open && t[1].close);
}
