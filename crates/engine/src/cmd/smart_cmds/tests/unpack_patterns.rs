use super::*;
use photocraft_doc::{Effect, Fill, FxCommon, Pattern};

fn patterned_source(sample: SampleType, fill: bool) -> Document {
    let mut doc = Document::new("Pattern source", Size::new(8, 8), ColorMode::Rgb, sample);
    let mut tile = Surface::new(doc.pixel_format());
    tile.fill_rect(Rect::new(0, 0, 2, 2), &[0.8, 0.1, 0.2, 1.0]);
    tile.fill_rect(Rect::new(0, 0, 1, 1), &[0.1, 0.6, 0.9, 1.0]);
    let pattern = Pattern::new("Original pattern", tile, 2, 2);
    let mut layer = Layer::raster("Patterned layer", doc.pixel_format());
    if fill {
        layer.content =
            LayerContent::Fill(Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) });
    } else {
        layer.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 7, 7), &[0.0, 0.0, 0.0, 1.0]);
        layer.effects.enabled = true;
        layer.effects.items.push(Effect::PatternOverlay {
            common: FxCommon { enabled: true, blend: BlendMode::Normal, opacity: 1.0 },
            name: pattern.name.clone(),
            id: pattern.id.clone(),
            scale: 1.0,
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        });
    }
    doc.patterns.push(pattern);
    doc.layers.push(layer);
    doc
}

fn embedded(source: &Document, psd: bool, patterns: Vec<Pattern>) -> Session {
    let (file_name, bytes) = if psd {
        ("source.psd", photocraft_io::export(source, "psd", &Default::default()).unwrap().bytes)
    } else {
        ("source.pcraft", encode_source(source).unwrap())
    };
    let source = decode_source(file_name, &bytes).unwrap();
    let pixels = photocraft_compose::flatten(&source).px.into_iter().flatten().collect::<Vec<_>>();
    let mut cache = Surface::new(source.pixel_format());
    cache.write_region(source.bounds(), &pixels);
    let mut layer = Layer::raster("Placed source", source.pixel_format());
    layer.content =
        LayerContent::Smart(SmartObject::new(SmartSource::Embedded { file_name: file_name.into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, Some(cache)));
    let mut doc = Document::new("Destination", source.size, source.mode, source.depth);
    doc.patterns = patterns;
    doc.layers.push(layer);
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

#[test]
fn convert_to_layers_keeps_embedded_pattern_overlay_and_fill_at_every_depth() {
    for sample in SampleType::ALL {
        for fill in [false, true] {
            for psd in [false, true] {
                let source = patterned_source(sample, fill);
                let source_before = source.clone();
                let mut s = embedded(&source, psd, Vec::new());
                let before = s.active().unwrap().doc.clone();
                let pixels = flat(&s);
                assert!(pixels.iter().any(|p| p[0] > 0.5 && p[1] < 0.3), "fixture has red pattern pixels");
                convert_to_layers(&mut s);
                let after = s.active().unwrap().doc.clone();
                assert_eq!(after.patterns.len(), 1, "{sample:?}, fill={fill}, psd={psd}");
                assert_eq!(after.patterns[0].id, source.patterns[0].id);
                assert_eq!(flat(&s), pixels, "{sample:?}, fill={fill}, psd={psd}");
                assert!(!matches!(active_layer(&s).content, LayerContent::Smart(_)));
                let bytes = encode_source(&after).unwrap();
                let reopened = decode_source("unpacked.pcraft", &bytes).unwrap();
                assert_eq!(reopened.patterns, after.patterns);
                assert_eq!(photocraft_compose::flatten(&reopened).px, pixels, "pattern survives reopen");
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, before, "undo restores layers and resources together");
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc, after);
                assert_eq!(flat(&s), pixels);
                assert_eq!(source, source_before, "source stays unchanged");
            }
        }
    }
}

#[test]
fn convert_to_layers_merges_pattern_ids_without_replacing_existing_resources() {
    let source = patterned_source(SampleType::U8, false);
    let pattern = source.patterns[0].clone();
    let mut other = pattern.clone();
    other.id = "destination-only".into();
    other.surface.fill_rect(other.rect(), &[0.0, 1.0, 0.0, 1.0]);
    // A shared display name must not keep the source's distinct id from being imported.
    for preloaded in [false, true] {
        let mut existing = vec![other.clone()];
        if preloaded {
            existing.push(pattern.clone());
        }
        let mut s = embedded(&source, false, existing.clone());
        let pixels = flat(&s);
        convert_to_layers(&mut s);
        assert_eq!(s.active().unwrap().doc.patterns, vec![other.clone(), pattern.clone()]);
        assert_eq!(flat(&s), pixels);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.patterns, existing);
    }
    // Like cross-document copy, conversion reuses an existing ID without overwriting its tile.
    other.id = pattern.id;
    let mut s = embedded(&source, false, vec![other.clone()]);
    convert_to_layers(&mut s);
    assert_eq!(s.active().unwrap().doc.patterns, vec![other]);
}

#[test]
fn convert_to_layers_keeps_patterns_used_inside_groups() {
    let mut source = patterned_source(SampleType::U8, false);
    let layer = source.layers.pop().unwrap();
    source.layers.push(Layer::group("Nested", vec![layer]));
    let mut s = embedded(&source, false, Vec::new());
    let pixels = flat(&s);
    convert_to_layers(&mut s);
    assert!(active_layer(&s).is_group());
    assert_eq!(s.active().unwrap().doc.patterns, source.patterns);
    assert_eq!(flat(&s), pixels);
}

#[test]
fn convert_to_layers_failure_does_not_import_embedded_patterns() {
    let source = patterned_source(SampleType::U8, false);
    let mut s = embedded(&source, false, Vec::new());
    s.execute("edit.transform.warp", json!({"style": "arc", "bend": 30})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.execute("layer.smartObjects.convertToLayers", json!({})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert!(s.active().unwrap().doc.patterns.is_empty());
}
