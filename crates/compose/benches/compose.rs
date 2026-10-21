//! `cargo bench -p photocraft-compose`
use criterion::{Criterion, criterion_group, criterion_main};
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerMask, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn filled(n: u32, seed: u8) -> Surface {
    let r = Rect::new(0, 0, n as i32, n as i32);
    let mut s = Surface::new(PixelFormat::RGBA8);
    let row: Vec<u8> = (0..n).flat_map(|x| [(x as u8).wrapping_mul(seed), (x / 3) as u8, seed, 200]).collect();
    let bytes: Vec<u8> = (0..n).flat_map(|_| row.iter().copied()).collect();
    s.write_interleaved(r, &bytes);
    s
}

/// 6016² document, 5 layers: background, masked, multiply, curves, screen (opacity/fill).
fn five_layer_doc(n: u32) -> Document {
    let mut d = Document::with_background("bench", Size::new(n, n), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut a = Layer::new("masked", LayerContent::Raster(filled(n, 3)));
    let mut m = LayerMask::reveal_all();
    m.surface.fill_rect(Rect::new(0, 0, n as i32 / 2, n as i32), &[0.3]);
    a.mask = Some(m);
    d.layers.push(a);
    let mut b = Layer::new("multiply", LayerContent::Raster(filled(n, 7)));
    b.blend = BlendMode::Multiply;
    d.layers.push(b);
    d.layers.push(Layer::new("curves", LayerContent::Adjustment(Adjustment::identity_curves())));
    let mut c = Layer::new("screen", LayerContent::Raster(filled(n, 11)));
    c.blend = BlendMode::Screen;
    c.opacity = 0.7;
    c.fill_opacity = 0.8;
    d.layers.push(c);
    d
}

/// 2048² document with 20 masked layers.
fn mask_heavy_doc() -> Document {
    let n = 2048;
    let mut d = Document::with_background("masks", Size::new(n, n), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    for i in 0..20u8 {
        let mut l = Layer::new(format!("l{i}"), LayerContent::Raster(filled(n, i + 2)));
        let mut m = LayerMask::hide_all();
        m.surface.fill_rect(Rect::new(i as i32 * 50, 0, i as i32 * 50 + 1200, n as i32), &[0.8]);
        l.mask = Some(m);
        l.opacity = 0.6;
        d.layers.push(l);
    }
    d
}

fn benches(c: &mut Criterion) {
    let doc = five_layer_doc(6016);
    let mut g = c.benchmark_group("compose");
    g.sample_size(10);
    g.bench_function("flatten_6016_5_layers", |b| b.iter(|| photocraft_compose::flatten(&doc)));    let masks = mask_heavy_doc();
    g.bench_function("flatten_2048_20_masked_layers", |b| b.iter(|| photocraft_compose::flatten(&masks)));
    let stroke = photocraft_paint::Stroke {
        brush: photocraft_paint::BrushSettings { size: 60.0, hardness: 0.5, ..Default::default() },
        points: (0..500).map(|i| photocraft_paint::StrokePoint::new(100.0 + i as f64 * 7.0, 1000.0 + (i as f64 * 0.05).sin() * 400.0, 1.0)).collect(),
    };
    let base = filled(4096, 5);
    let mut sel = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
    sel.fill_rect(Rect::new(0, 0, 2000, 4096), &[0.5]);
    g.bench_function("paint_stroke_500pt_60px_with_selection", |b| {
        b.iter(|| {
            let mut s = base.clone();
            photocraft_paint::apply_stroke(&mut s, &stroke, Some(&sel), false)
        })
    });
    gradient_drag_frame(&mut g);
    g.finish();
}

/// One live-gradient-drag frame (`gradient_ui::display_doc` + canvas composite): a 24 MP photo
/// document with a full-canvas gradient fill layer on top, composited at display size with a
/// different gradient angle every iteration (a drag never repeats a frame).
fn photo_doc_24mp() -> Document {
    let (w, h) = (6000u32, 4000u32);
    let mut doc = Document::with_background("photo", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers.push(Layer::new("photo", LayerContent::Raster(filled(6000, 9))));
    doc
}

fn gradient_fill(angle: f64) -> photocraft_doc::Fill {
    use photocraft_doc::GradientStyle;
    let stops = vec![(0.0, Color::rgb(0.08, 0.16, 0.78)), (0.5, Color::rgb(0.78, 0.16, 0.47)), (1.0, Color::rgb(0.94, 0.86, 0.31))];
    photocraft_doc::Fill::gradient(stops, angle as f32, 100.0, GradientStyle::Linear, false)
}

fn gradient_drag_frame(g: &mut criterion::BenchmarkGroup<criterion::measurement::WallTime>) {
    let doc = photo_doc_24mp();
    let mut angle = 0.0;
    g.bench_function("gradient_drag_frame_24mp_display", |b| {
        b.iter(|| {
            angle += 7.0;
            let mut frame = doc.clone();
            frame.layers.push(Layer::new("gradient", LayerContent::Fill(gradient_fill(angle))));
            photocraft_compose::render_reduced(&frame, 1600, 1067)
        })
    });
    // Same document without the gradient layer: the composite baseline the gradient adds to.
    g.bench_function("gradient_drag_baseline_24mp_display", |b| {
        b.iter(|| photocraft_compose::render_reduced(&doc, 1600, 1067))
    });
    // Split composite: the static part once (cached across iterations), the gradient group
    // fresh — the live-drag path (`render_reduced_split`).
    g.bench_function("gradient_drag_split_24mp_display", |b| {
        let mut frame0 = doc.clone();
        frame0.layers.push(Layer::new("gradient", LayerContent::Fill(gradient_fill(0.0))));
        let gid = frame0.layers.last().map(|l| l.id).unwrap();
        let k = photocraft_compose::preview_split(&frame0, gid).unwrap();
        let mut base: Option<photocraft_compose::Buffer> = None;
        let mut angle = 0.0;
        b.iter(|| {
            angle += 7.0;
            let mut frame = doc.clone();
            frame.layers.push(Layer::new("gradient", LayerContent::Fill(gradient_fill(angle))));
            let (full, b) = photocraft_compose::render_reduced_split(&frame, k, 1600, 1067, base.take());
            base = Some(b);
            full
        })
    });
    // Live-drag preview path: exact cached base, reduced-space top (`render_reduced_preview`).
    g.bench_function("gradient_drag_preview_24mp_display", |b| {
        let mut frame0 = doc.clone();
        frame0.layers.push(Layer::new("gradient", LayerContent::Fill(gradient_fill(0.0))));
        let gid = frame0.layers.last().map(|l| l.id).unwrap();
        let k = photocraft_compose::preview_split(&frame0, gid).unwrap();
        let mut base: Option<photocraft_compose::Buffer> = None;
        let mut angle = 0.0;
        b.iter(|| {
            angle += 7.0;
            let mut frame = doc.clone();
            frame.layers.push(Layer::new("gradient", LayerContent::Fill(gradient_fill(angle))));
            let (full, b) = photocraft_compose::render_reduced_preview(&frame, k, 1600, 1067, base.take()).unwrap();
            base = Some(b);
            full
        })
    });
}

criterion_group!(compose_benches, benches);
criterion_main!(compose_benches);
