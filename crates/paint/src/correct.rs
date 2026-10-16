//! Hold-to-correct: fit a finished freehand stroke to a clean primitive.
//!
//! When the pointer rests at the end of a stroke (Clip Studio / QuickShape behaviour), the raw
//! points are classified here — line, ellipse/circle, triangle, rectangle/square, or a smoothed
//! polyline fallback — and re-rendered from the fit. Pure geometry on point lists, so every path
//! is unit-testable; hostile input (NaN, infinities, duplicates, fewer than two distinct points)
//! yields `None` (keep the raw stroke), never a panic.

use crate::StrokePoint;

/// A clean primitive the raw stroke was recognised as.
#[derive(Clone, Debug, PartialEq)]
pub enum Correction {
    /// Straight segment from `a` to `b`.
    Line { a: [f64; 2], b: [f64; 2] },
    /// Axis-aligned-or-rotated ellipse (`rx == ry` is a circle); `rot` in radians.
    Ellipse { center: [f64; 2], rx: f64, ry: f64, rot: f64 },
    /// Three corners in stroke order.
    Triangle([[f64; 2]; 3]),
    /// Four corners in stroke order (squares included).
    Rect([[f64; 2]; 4]),
    /// Smoothed polyline fallback for open strokes that fit nothing else.
    Smooth(Vec<[f64; 2]>),
}

/// Maximum relative deviation for a line fit (max distance / segment length).
const LINE_TOL: f64 = 0.06;
/// A stroke counts as closed when its ends meet within this fraction of its length.
const CLOSED_TOL: f64 = 0.15;
/// Douglas–Peucker epsilon as a fraction of path length for corner detection.
const CORNER_EPS: f64 = 0.02;
/// Maximum normalized radial residual (std of unit-circle radius in the PCA frame) for a
/// circle/ellipse fit. Works at any eccentricity; polygons are tested first, so a square
/// never reaches it, and blobs fail it.
const ELLIPSE_TOL: f64 = 0.12;
/// Corner angles must clear these bounds (degrees) for triangle/rect fits.
const MIN_CORNER_DEG: f64 = 20.0;
const MAX_CORNER_DEG: f64 = 160.0;
/// A rectangle's opposite sides must agree within this fraction.
const RECT_SIDE_TOL: f64 = 0.30;
/// A rectangle's corners must clear 90 degrees by this margin.
const RECT_ANGLE_TOL_DEG: f64 = 30.0;

fn finite(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Total polyline length; degenerate (empty/single-point) input yields 0.
pub fn path_length(pts: &[[f64; 2]]) -> f64 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// Deduplicated finite points; `None` when fewer than two distinct points remain.
fn clean(raw: &[StrokePoint]) -> Option<Vec<[f64; 2]>> {
    let mut pts = Vec::with_capacity(raw.len());
    for p in raw {
        let q = [p.x, p.y];
        if finite(q) && pts.last() != Some(&q) {
            pts.push(q);
        }
    }
    if pts.len() >= 2 { Some(pts) } else { None }
}

/// Centroid and the unit principal axis of `pts` (2×2 PCA, closed form).
fn principal(pts: &[[f64; 2]]) -> Option<([f64; 2], [f64; 2], [f64; 2])> {
    let n = pts.len() as f64;
    if n == 0.0 {
        return None;
    }
    let (mut mx, mut my) = (0.0, 0.0);
    for p in pts {
        mx += p[0];
        my += p[1];
    }
    let c = [mx / n, my / n];
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for p in pts {
        let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    // Eigenvector of the larger eigenvalue of [[sxx, sxy], [sxy, syy]].
    let trace = sxx + syy;
    let det = sxx * syy - sxy * sxy;
    let disc = (trace * trace / 4.0 - det).max(0.0).sqrt();
    let lambda = trace / 2.0 + disc;
    let mut axis = [lambda - syy, sxy];
    if axis[0].hypot(axis[1]) < 1e-12 {
        axis = [sxy, lambda - sxx];
    }
    let len = axis[0].hypot(axis[1]);
    if len <= 1e-12 {
        return None;
    }
    let axis = [axis[0] / len, axis[1] / len];
    let minor = [-axis[1], axis[0]];
    Some((c, axis, minor))
}

/// Total-least-squares line through `pts`; `None` on degenerate input.
fn fit_line(pts: &[[f64; 2]]) -> Option<([f64; 2], [f64; 2])> {
    let (c, axis, _) = principal(pts)?;
    // Project the extremes onto the axis for stable endpoints.
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for p in pts {
        let t = (p[0] - c[0]) * axis[0] + (p[1] - c[1]) * axis[1];
        min = min.min(t);
        max = max.max(t);
    }
    if max <= min {
        return None;
    }
    Some(([c[0] + axis[0] * min, c[1] + axis[1] * min], [c[0] + axis[0] * max, c[1] + axis[1] * max]))
}

fn max_line_dev(pts: &[[f64; 2]], a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len = dx.hypot(dy);
    if len <= 0.0 {
        return f64::INFINITY;
    }
    pts.iter()
        .map(|p| ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len)
        .fold(0.0, f64::max)
}

/// Iterative Douglas–Peucker; returns the kept indices. Bounded work list, no recursion.
fn douglas_peucker(pts: &[[f64; 2]], eps: f64) -> Vec<usize> {
    let n = pts.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    // Malformed input degrades to the endpoints: cap the work at a linear budget.
    let mut budget = 4 * n + 16;
    while let Some((a, b)) = stack.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (pts[a], pts[b]);
        let dx = pb[0] - pa[0];
        let dy = pb[1] - pa[1];
        let len = dx.hypot(dy);
        let mut best = 0.0;
        let mut idx = a;
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len > 0.0 {
                ((p[0] - pa[0]) * dy - (p[1] - pa[1]) * dx).abs() / len
            } else {
                dist(*p, pa)
            };
            if d > best {
                best = d;
                idx = i;
            }
        }
        if best > eps {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    keep.into_iter().enumerate().filter(|(_, k)| *k).map(|(i, _)| i).collect()
}

/// Corner detection on a closed ring: close the loop exactly, split it at the point
/// farthest from the start, and run Douglas–Peucker on the two open halves. Plain DP on the
/// open ring would keep both ends (which straddle one corner) as two separate corners.
fn closed_corners(ring: &[[f64; 2]], eps: f64) -> Vec<usize> {
    let n = ring.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut loop_pts = ring.to_vec();
    loop_pts.push(ring[0]);
    let mut m = 0;
    let mut best = 0.0;
    for (i, p) in loop_pts.iter().enumerate().take(n).skip(1) {
        let d = dist(*p, loop_pts[0]);
        if d > best {
            best = d;
            m = i;
        }
    }
    if m == 0 {
        return douglas_peucker(ring, eps);
    }
    let mut keep = douglas_peucker(&loop_pts[0..=m], eps);
    for i in douglas_peucker(&loop_pts[m..], eps).into_iter().skip(1) {
        // The halves' shared end: absolute index n duplicates corner 0, drop it.
        if m + i >= n {
            break;
        }
        keep.push(m + i);
    }
    decimate(ring, &mut keep, eps);
    keep
}

/// Drop kept corners the polyline does not need: a corner goes when the arc skipping it
/// stays within `eps` of the chord. Cyclic (the loop is closed), so a stroke starting
/// mid-edge loses its spurious endpoints, and one rounded hand-drawn corner stops
/// counting as two. Never decimates below a triangle.
fn decimate(ring: &[[f64; 2]], keep: &mut Vec<usize>, eps: f64) {
    if eps <= 0.0 {
        return;
    }
    loop {
        if keep.len() <= 3 {
            return;
        }
        let mut dropped = false;
        let mut k = 0;
        while k < keep.len() {
            let prev = keep[(k + keep.len() - 1) % keep.len()];
            let next = keep[(k + 1) % keep.len()];
            if arc_deviation(ring, prev, next) <= eps {
                keep.remove(k);
                dropped = true;
            } else {
                k += 1;
            }
        }
        if !dropped {
            return;
        }
    }
}

/// Maximum distance from the ring arc between corners `a` and `b` (forward, wrapping) to
/// their chord. Indices equal or adjacent along the ring mean an empty arc.
fn arc_deviation(ring: &[[f64; 2]], a: usize, b: usize) -> f64 {
    let n = ring.len();
    if n == 0 || a >= n || b >= n {
        return f64::INFINITY;
    }
    let (pa, pb) = (ring[a], ring[b]);
    let dx = pb[0] - pa[0];
    let dy = pb[1] - pa[1];
    let len = dx.hypot(dy);
    let mut i = (a + 1) % n;
    let mut best: f64 = 0.0;
    while i != b {
        let p = ring[i];
        let d = if len > 0.0 {
            ((p[0] - pa[0]) * dy - (p[1] - pa[1]) * dx).abs() / len
        } else {
            dist(p, pa)
        };
        best = best.max(d);
        i = (i + 1) % n;
        if i == (a + 1) % n {
            break;
        }
    }
    best
}

fn corner_angle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let (u0, u1) = (a[0] - b[0], a[1] - b[1]);
    let (v0, v1) = (c[0] - b[0], c[1] - b[1]);
    let (lu, lv) = (u0.hypot(u1), v0.hypot(v1));
    if !(lu > 0.0 && lv > 0.0) {
        return 0.0;
    }
    let cos = ((u0 * v0 + u1 * v1) / (lu * lv)).clamp(-1.0, 1.0);
    cos.acos().to_degrees()
}

/// Two Chaikin corner-cutting passes; endpoints preserved.
fn chaikin(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut cur: Vec<[f64; 2]> = pts.to_vec();
    for _ in 0..2 {
        if cur.len() < 2 {
            break;
        }
        let mut next = Vec::with_capacity(cur.len() * 2);
        next.push(cur[0]);
        for w in cur.windows(2) {
            let (a, b) = (w[0], w[1]);
            next.push([a[0] * 0.75 + b[0] * 0.25, a[1] * 0.75 + b[1] * 0.25]);
            next.push([a[0] * 0.25 + b[0] * 0.75, a[1] * 0.25 + b[1] * 0.75]);
        }
        next.push(cur[cur.len() - 1]);
        cur = next;
    }
    cur
}

/// Classify a finished freehand stroke; `None` means "leave the raw stroke alone".
pub fn fit(raw: &[StrokePoint]) -> Option<Correction> {
    let pts = clean(raw)?;
    let len = path_length(&pts);
    if len <= 0.0 {
        return None;
    }
    let first = pts[0];
    let last = pts[pts.len() - 1];
    let closed = dist(first, last) <= CLOSED_TOL * len;

    if !closed {
        if let Some((a, b)) = fit_line(&pts) {
            let seg = dist(a, b);
            if seg > 0.0 && max_line_dev(&pts, a, b) / seg <= LINE_TOL {
                return Some(Correction::Line { a, b });
            }
        }
        return Some(Correction::Smooth(chaikin(&pts)));
    }

    // Closed: drop the duplicated endpoint for the fits below. The ring's ends
    // can still straddle the same corner (sampling wobble), so trim points
    // within corner tolerance of the start as well.
    let mut ring = &pts[..pts.len() - 1];
    while ring.len() > 1 && dist(ring[0], ring[ring.len() - 1]) <= CORNER_EPS * len {
        ring = &ring[..ring.len() - 1];
    }
    if ring.len() < 8 {
        return None;
    }
    // Polygon before ellipse: a wobbly square can pass the radial test, while a
    // genuine ellipse never survives corner detection with 3–4 corners.
    let keep = closed_corners(ring, CORNER_EPS * len);
    if keep.len() == 3 {
        let c: Vec<[f64; 2]> = keep.iter().map(|&i| ring[i]).collect();
        let ok = (0..3).all(|k| {
            let a = corner_angle(c[(k + 2) % 3], c[k], c[(k + 1) % 3]);
            (MIN_CORNER_DEG..=MAX_CORNER_DEG).contains(&a)
        });
        if ok {
            return Some(Correction::Triangle([c[0], c[1], c[2]]));
        }
    } else if keep.len() == 4 {
        let c: Vec<[f64; 2]> = keep.iter().map(|&i| ring[i]).collect();
        let sides: Vec<f64> = (0..4).map(|k| dist(c[k], c[(k + 1) % 4])).collect();
        let angles_ok = (0..4).all(|k| {
            let a = corner_angle(c[(k + 3) % 4], c[k], c[(k + 1) % 4]);
            (a - 90.0).abs() <= RECT_ANGLE_TOL_DEG
        });
        let sides_ok = [0, 1].iter().all(|&k| {
            let (x, y) = (sides[k], sides[(k + 2) % 4]);
            if x + y <= 0.0 {
                return false;
            }
            (x - y).abs() / (x + y) * 2.0 <= RECT_SIDE_TOL
        });
        if angles_ok && sides_ok {
            return Some(Correction::Rect([c[0], c[1], c[2], c[3]]));
        }
    }
    // Circle / ellipse in the PCA frame: semi-axes are the extents, and the fit holds
    // when the normalized radii cluster near 1. Unlike a radial-dispersion gate this works
    // at any eccentricity; polygons were tested above, so squares never reach it.
    // (Pentagons and hexagons can pass it and come out ellipses: accepted, rare.)
    let (center, axis, minor) = principal(ring).unwrap_or(([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]));
    let (mut ext_a, mut ext_m) = (0.0f64, 0.0f64);
    let mut dxs = Vec::with_capacity(ring.len());
    for p in ring {
        let dx = [p[0] - center[0], p[1] - center[1]];
        dxs.push(dx);
        ext_a = ext_a.max((dx[0] * axis[0] + dx[1] * axis[1]).abs());
        ext_m = ext_m.max((dx[0] * minor[0] + dx[1] * minor[1]).abs());
    }
    if ext_a > 0.0 && ext_m > 0.0 {
        let mut mean = 0.0;
        let mut rho = Vec::with_capacity(ring.len());
        for d in &dxs {
            let r = ((d[0] * axis[0] + d[1] * axis[1]) / ext_a).powi(2)
                + ((d[0] * minor[0] + d[1] * minor[1]) / ext_m).powi(2);
            let r = r.max(0.0).sqrt();
            rho.push(r);
            mean += r;
        }
        mean /= rho.len().max(1) as f64;
        let var = rho.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / rho.len().max(1) as f64;
        if var.sqrt() <= ELLIPSE_TOL {
            return Some(Correction::Ellipse {
                center,
                rx: ext_a,
                ry: ext_m,
                rot: axis[1].atan2(axis[0]),
            });
        }
    }
    None
}

/// Rebuild a fitted primitive between a fixed start and a moved end (hold-to-correct
/// move phase: the stroke start stays put, the end follows the pointer). Lines span the two
/// points; rectangles and ellipses take them as opposite bbox corners; triangles affine-map
/// the trigger shape from its bbox to the new one. `None` on a degenerate span. `Smooth`
/// has no clean primitive: the caller refits the adjusted raw points instead.
pub fn stretch(base: &Correction, start: [f64; 2], end: [f64; 2]) -> Option<Correction> {
    if !(start.iter().all(|c| c.is_finite()) && end.iter().all(|c| c.is_finite())) {
        return None;
    }
    if dist(start, end) < 2.0 {
        return None;
    }
    let bbox = |pts: &[[f64; 2]]| {
        let mut min = [f64::INFINITY, f64::INFINITY];
        let mut max = [f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in pts {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        (min, max)
    };
    match base {
        Correction::Line { .. } => Some(Correction::Line { a: start, b: end }),
        Correction::Rect(_) => {
            let (lo, hi) = ( [start[0].min(end[0]), start[1].min(end[1])], [start[0].max(end[0]), start[1].max(end[1])] );
            Some(Correction::Rect([[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]]))
        }
        Correction::Ellipse { .. } => Some(Correction::Ellipse {
            center: [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0],
            rx: (end[0] - start[0]).abs() / 2.0,
            ry: (end[1] - start[1]).abs() / 2.0,
            rot: 0.0,
        }),
        Correction::Triangle(c) => {
            let (omin, omax) = bbox(c);
            let (nmin, nmax) = ( [start[0].min(end[0]), start[1].min(end[1])], [start[0].max(end[0]), start[1].max(end[1])] );
            let map = |p: [f64; 2]| {
                let mut q = p;
                for i in 0..2 {
                    let span = omax[i] - omin[i];
                    q[i] = if span > 1e-9 {
                        nmin[i] + (p[i] - omin[i]) / span * (nmax[i] - nmin[i])
                    } else {
                        (nmin[i] + nmax[i]) / 2.0
                    };
                }
                q
            };
            Some(Correction::Triangle([map(c[0]), map(c[1]), map(c[2])]))
        }
        Correction::Smooth(_) => None,
    }
}

/// Sample `count` points along a correction for rendering.
pub fn sample(c: &Correction, count: usize) -> Vec<[f64; 2]> {
    let n = count.clamp(2, 512);
    match c {
        Correction::Line { a, b } => (0..n)
            .map(|i| {
                let t = i as f64 / (n - 1) as f64;
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
            })
            .collect(),
        Correction::Ellipse { center, rx, ry, rot } => {
            let (s, co) = rot.sin_cos();
            (0..=n)
                .map(|i| {
                    let t = i as f64 / n as f64 * std::f64::consts::TAU;
                    let (x, y) = (rx * t.cos(), ry * t.sin());
                    [center[0] + x * co - y * s, center[1] + x * s + y * co]
                })
                .collect()
        }
        Correction::Triangle(c) => poly_sample(&[c[0], c[1], c[2], c[0]], n),
        Correction::Rect(c) => poly_sample(&[c[0], c[1], c[2], c[3], c[0]], n),
        Correction::Smooth(pts) => poly_sample(pts, n),
    }
}

fn poly_sample(pts: &[[f64; 2]], n: usize) -> Vec<[f64; 2]> {
    let total = path_length(pts);
    if total <= 0.0 || pts.is_empty() {
        return pts.to_vec();
    }
    // Cumulative lengths for arc-length walking.
    let mut cum = Vec::with_capacity(pts.len());
    let mut acc = 0.0;
    cum.push(0.0);
    for w in pts.windows(2) {
        acc += dist(w[0], w[1]);
        cum.push(acc);
    }
    (0..n)
        .map(|i| {
            let target = total * i as f64 / (n - 1).max(1) as f64;
            let mut k = 0;
            while k + 1 < cum.len() && cum[k + 1] < target {
                k += 1;
            }
            let next = (k + 1).min(pts.len() - 1);
            let seg = cum[next] - cum[k];
            let t = if seg > 0.0 { (target - cum[k]) / seg } else { 0.0 };
            let (a, b) = (pts[k], pts[next]);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        })
        .collect()
}

/// Build renderable points from a correction, carrying the raw stroke's pressure
/// (interpolated by arc length) and stylus state. Non-finite stylus values fall back to
/// neutral ones: a hand-edited or fuzzed point must degrade pixels, never poison the renderer.
pub fn stroke_points(c: &Correction, raw: &[StrokePoint]) -> Vec<StrokePoint> {
    let finite_raw: Vec<&StrokePoint> =
        raw.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).collect();
    if finite_raw.is_empty() {
        return Vec::new();
    }
    let first = *finite_raw[0];
    let last = *finite_raw[finite_raw.len() - 1];
    let sane_pressure = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
    let sane_tilt = |v: f32| if v.is_finite() { v.clamp(-90.0, 90.0) } else { 0.0 };
    let sane_rot = |v: f32| if v.is_finite() { v.rem_euclid(360.0) } else { 0.0 };
    let sane_wheel = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    // Arc-length parameter of each raw point for pressure lookup.
    let mut cum = Vec::with_capacity(finite_raw.len());
    let mut acc = 0.0;
    cum.push(0.0);
    for w in finite_raw.windows(2) {
        acc += dist([w[0].x, w[0].y], [w[1].x, w[1].y]);
        cum.push(acc);
    }
    let total = acc;
    let at = |s: f64| -> f32 {
        if total <= 0.0 {
            return sane_pressure(first.pressure);
        }
        let mut k = 0;
        while k + 1 < cum.len() && cum[k + 1] < s {
            k += 1;
        }
        let next = (k + 1).min(finite_raw.len() - 1);
        let seg = cum[next] - cum[k];
        let t = if seg > 0.0 { ((s - cum[k]) / seg) as f32 } else { 0.0 };
        sane_pressure(finite_raw[k].pressure + (finite_raw[next].pressure - finite_raw[k].pressure) * t.clamp(0.0, 1.0))
    };
    // A straight line needs only its endpoints (dab spacing lays the coverage);
    // shapes sample along the perimeter. Fewer points = cheaper move-phase re-renders.
    let count = match c {
        Correction::Line { .. } => 2,
        _ => finite_raw.len().clamp(8, 256),
    };
    let geo = sample(c, count);
    let glen = path_length(&geo);
    let count = geo.len().max(1);
    let t0 = first.time;
    let t1 = last.time;
    let mut gacc = 0.0;
    let mut prev: Option<[f64; 2]> = None;
    geo.into_iter()
        .enumerate()
        .map(|(i, g)| {
            if let Some(p) = prev {
                gacc += dist(p, g);
            }
            prev = Some(g);
            StrokePoint {
                x: g[0],
                y: g[1],
                pressure: at(if glen > 0.0 { gacc / glen * total } else { 0.0 }),
                tilt_x: sane_tilt(first.tilt_x),
                tilt_y: sane_tilt(first.tilt_y),
                rotation: sane_rot(first.rotation),
                wheel: sane_wheel(first.wheel),
                time: if t1 >= t0 { t0 + (t1 - t0) * i as f64 / count as f64 } else { t0 },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(coords: &[[f64; 2]]) -> Vec<StrokePoint> {
        coords
            .iter()
            .enumerate()
            .map(|(i, c)| StrokePoint {
                x: c[0],
                y: c[1],
                pressure: 0.8,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                wheel: 0.0,
                time: i as f64 * 8.0,
            })
            .collect()
    }

    /// Deterministic wobble so the tests don't depend on randomness.
    fn wobble(i: usize) -> f64 {
        (((i * 2654435761) >> 8) % 100) as f64 / 100.0 - 0.5
    }

    #[test]
    fn wobbly_line_fits_line() {
        let raw: Vec<[f64; 2]> = (0..40).map(|i| [i as f64 * 5.0, wobble(i) * 6.0]).collect();
        let c = fit(&pts(&raw)).expect("a line fits");
        assert!(matches!(c, Correction::Line { .. }), "got {c:?}");
        if let Correction::Line { a, b } = c {
            assert!(dist(a, b) > 150.0, "endpoints span the stroke");
        }
    }

    #[test]
    fn circle_fits_ellipse() {
        let raw: Vec<[f64; 2]> = (0..64)
            .map(|i| {
                let t = i as f64 / 64.0 * std::f64::consts::TAU;
                [100.0 + 60.0 * t.cos() + wobble(i) * 3.0, 100.0 + 60.0 * t.sin() + wobble(i + 7) * 3.0]
            })
            .collect();
        let c = fit(&pts(&raw)).expect("a circle fits");
        assert!(matches!(c, Correction::Ellipse { .. }), "got {c:?}");
        if let Correction::Ellipse { center, rx, ry, .. } = c {
            assert!((center[0] - 100.0).abs() < 5.0 && (center[1] - 100.0).abs() < 5.0, "center {center:?}");
            assert!((rx - 60.0).abs() < 6.0 && (ry - 60.0).abs() < 6.0, "radii {rx} {ry}");
        }
    }

    #[test]
    fn triangle_fits_triangle() {
        let corners = [[10.0, 10.0], [190.0, 20.0], [100.0, 170.0]];
        let mut raw = Vec::new();
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            for i in 0..=20 {
                let t = i as f64 / 20.0;
                raw.push([a[0] + (b[0] - a[0]) * t + wobble(raw.len()) * 3.0, a[1] + (b[1] - a[1]) * t + wobble(raw.len() + 3) * 3.0]);
            }
        }
        raw.push(raw[0]);
        let c = fit(&pts(&raw)).expect("a triangle fits");
        assert!(matches!(c, Correction::Triangle(_)), "got {c:?}");
    }

    #[test]
    fn square_fits_rect() {
        let corners = [[20.0, 20.0], [180.0, 20.0], [180.0, 180.0], [20.0, 180.0]];
        let mut raw = Vec::new();
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            for i in 0..=20 {
                let t = i as f64 / 20.0;
                raw.push([a[0] + (b[0] - a[0]) * t + wobble(raw.len()) * 3.0, a[1] + (b[1] - a[1]) * t + wobble(raw.len() + 5) * 3.0]);
            }
        }
        raw.push(raw[0]);
        let c = fit(&pts(&raw)).expect("a square fits");
        assert!(matches!(c, Correction::Rect(_)), "got {c:?}");
    }

    #[test]
    fn scribble_falls_back_to_smooth() {
        // An S-curve: open, not a line.
        let raw: Vec<[f64; 2]> =
            (0..50).map(|i| [i as f64 * 4.0, 100.0 + 60.0 * ((i as f64 / 50.0 * 4.0).sin())]).collect();
        let c = fit(&pts(&raw)).expect("a fallback exists");
        assert!(matches!(c, Correction::Smooth(_)), "got {c:?}");
    }

    #[test]
    fn degenerate_input_yields_none_or_empty() {
        assert!(fit(&[]).is_none(), "empty");
        assert!(fit(&pts(&[[1.0, 2.0]])).is_none(), "single point");
        assert!(fit(&pts(&[[1.0, 2.0], [1.0, 2.0], [1.0, 2.0]])).is_none(), "coincident");
        let nan = vec![
            StrokePoint { x: f64::NAN, y: 0.0, pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, wheel: 0.0, time: 0.0 },
            StrokePoint { x: f64::INFINITY, y: f64::NEG_INFINITY, pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, wheel: 0.0, time: 1.0 },
        ];
        assert!(fit(&nan).is_none(), "non-finite");
        assert!(stroke_points(&Correction::Line { a: [0.0, 0.0], b: [1.0, 1.0] }, &[]).is_empty());
    }

    #[test]
    fn stroke_points_carry_pressure_and_time() {
        let raw: Vec<[f64; 2]> = (0..20).map(|i| [i as f64 * 10.0, 0.0]).collect();
        let mut input = pts(&raw);
        for (i, p) in input.iter_mut().enumerate() {
            p.pressure = i as f32 / 19.0;
        }
        let c = fit(&input).expect("line");
        let out = stroke_points(&c, &input);
        assert!(!out.is_empty());
        assert!((out[0].pressure - 0.0).abs() < 0.05, "pressure starts low");
        assert!((out[out.len() - 1].pressure - 1.0).abs() < 0.05, "pressure ends high");
        assert!(out.windows(2).all(|w| w[1].time >= w[0].time), "time is monotone");
    }
}

#[cfg(test)]
mod robustness_tests {
    use super::super::*;
    use super::*;

    fn wob(i: usize) -> f64 {
        (((i * 2654435761) >> 8) % 100) as f64 / 100.0 - 0.5
    }

    fn mk(coords: &[[f64; 2]]) -> Vec<StrokePoint> {
        coords
            .iter()
            .enumerate()
            .map(|(i, c)| StrokePoint {
                x: c[0],
                y: c[1],
                pressure: 0.8,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                wheel: 0.0,
                time: i as f64 * 8.0,
            })
            .collect()
    }

    #[test]
    fn rounded_square_corners_merge_to_rect() {
        // A square whose corners are rounded (radius 12): DP sees extra corners.
        let corners = [[20.0, 20.0], [180.0, 20.0], [180.0, 180.0], [20.0, 180.0]];
        let mut raw = Vec::new();
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            for i in 0..=20 {
                let t = i as f64 / 20.0;
                let mut p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                // Round the corner at the end of each edge: pull toward the inside.
                let d_end = (1.0 - t) * 160.0;
                if d_end < 12.0 {
                    let pull = (12.0 - d_end) * 0.5;
                    let cx = (b[0] + corners[(k + 2) % 4][0]) / 2.0;
                    let cy = (b[1] + corners[(k + 2) % 4][1]) / 2.0;
                    p[0] += (cx - p[0]) * pull / 12.0;
                    p[1] += (cy - p[1]) * pull / 12.0;
                }
                p[0] += wob(raw.len()) * 4.0;
                p[1] += wob(raw.len() + 11) * 4.0;
                raw.push(p);
            }
        }
        raw.push(raw[0]);
        let c = fit(&mk(&raw)).expect("a rounded square fits");
        assert!(matches!(c, Correction::Rect(_)), "got {c:?}");
    }

    #[test]
    fn loose_circle_still_fits_ellipse() {
        // ±9 px wobble on r=60: past the old strict radial gate.
        let raw: Vec<[f64; 2]> = (0..64)
            .map(|i| {
                let t = i as f64 / 64.0 * std::f64::consts::TAU;
                [100.0 + 60.0 * t.cos() + wob(i) * 18.0, 100.0 + 60.0 * t.sin() + wob(i + 7) * 18.0]
            })
            .collect();
        let c = fit(&mk(&raw)).expect("a loose circle fits");
        assert!(matches!(c, Correction::Ellipse { .. }), "got {c:?}");
    }
}

#[cfg(test)]
mod edge_tests {
    use super::super::*;
    use super::*;

    /// Deterministic xorshift: property tests without a `rand` dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (u64::MAX >> 11) as f64
        }
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }
    }

    fn mk(coords: &[[f64; 2]]) -> Vec<StrokePoint> {
        coords
            .iter()
            .enumerate()
            .map(|(i, c)| StrokePoint {
                x: c[0],
                y: c[1],
                pressure: 0.8,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                wheel: 0.0,
                time: i as f64 * 8.0,
            })
            .collect()
    }

    fn rot(p: [f64; 2], deg: f64) -> [f64; 2] {
        let (s, c) = (deg.to_radians().sin(), deg.to_radians().cos());
        [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
    }

    fn draw_poly(corners: &[[f64; 2]], per_edge: usize, rng: &mut Rng, wob: f64, close: bool) -> Vec<[f64; 2]> {
        let mut raw = Vec::new();
        let n = corners.len();
        for k in 0..n {
            let (a, b) = (corners[k], corners[(k + 1) % n]);
            // Open polylines stop at the last corner instead of wrapping.
            if !close && k == n - 1 {
                break;
            }
            for i in 0..=per_edge {
                let t = i as f64 / per_edge as f64;
                raw.push([
                    a[0] + (b[0] - a[0]) * t + (rng.next() - 0.5) * 2.0 * wob,
                    a[1] + (b[1] - a[1]) * t + (rng.next() - 0.5) * 2.0 * wob,
                ]);
            }
        }
        if close {
            raw.push(raw[0]);
        }
        raw
    }

    fn circle(cx: f64, cy: f64, r: f64, count: usize, rng: &mut Rng, wob: f64) -> Vec<[f64; 2]> {
        (0..count)
            .map(|i| {
                let t = i as f64 / count as f64 * std::f64::consts::TAU;
                [
                    cx + r * t.cos() + (rng.next() - 0.5) * 2.0 * wob,
                    cy + r * t.sin() + (rng.next() - 0.5) * 2.0 * wob,
                ]
            })
            .collect()
    }

    #[test]
    fn rotated_shapes_classify() {
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        // A diamond is a square rotated 45°; rotation must not matter.
        let diamond: Vec<[f64; 2]> =
            [[0.0, -100.0], [100.0, 0.0], [0.0, 100.0], [-100.0, 0.0]].iter().map(|&p| rot(p, 13.0)).collect();
        let c = fit(&mk(&draw_poly(&diamond, 20, &mut rng, 2.0, true))).expect("diamond fits");
        assert!(matches!(c, Correction::Rect(_)), "got {c:?}");
        // A triangle pointing anywhere classifies.
        let tri: Vec<[f64; 2]> =
            [[0.0, -90.0], [85.0, 60.0], [-85.0, 60.0]].iter().map(|&p| rot(p, 200.0)).collect();
        let c = fit(&mk(&draw_poly(&tri, 20, &mut rng, 2.0, true))).expect("rotated triangle fits");
        assert!(matches!(c, Correction::Triangle(_)), "got {c:?}");
        // A rotated ellipse keeps its rotation.
        let raw: Vec<[f64; 2]> = (0..64)
            .map(|i| {
                let t = i as f64 / 64.0 * std::f64::consts::TAU;
                let p = [80.0 * t.cos(), 40.0 * t.sin()];
                let q = rot(p, 30.0);
                [100.0 + q[0], 100.0 + q[1]]
            })
            .collect();
        let c = fit(&mk(&raw)).expect("rotated ellipse fits");
        if let Correction::Ellipse { rx, ry, rot: r, .. } = c {
            assert!((rx - 80.0).abs() < 8.0 && (ry - 40.0).abs() < 8.0, "axes {rx} {ry}");
            let deg = r.to_degrees().rem_euclid(180.0);
            assert!((deg - 30.0).abs() < 10.0 || (deg - 210.0).abs() < 10.0, "rotation {deg}");
        } else {
            panic!("got {c:?}");
        }
    }

    #[test]
    fn orientation_and_start_point_do_not_matter() {
        let mut rng = Rng(0xdead_beef_cafe);
        // Counter-clockwise square starting mid-edge.
        let corners = [[20.0, 20.0], [20.0, 180.0], [180.0, 180.0], [180.0, 20.0]];
        let mut raw = draw_poly(&corners, 16, &mut rng, 2.0, true);
        raw.rotate_left(25);
        raw.push(raw[0]);
        let c = fit(&mk(&raw)).expect("ccw mid-edge square fits");
        assert!(matches!(c, Correction::Rect(_)), "got {c:?}");
    }

    #[test]
    fn tiny_and_sparse_shapes() {
        let mut rng = Rng(0x777);
        // A 12-point triangle: sparse but valid.
        let tri = [[0.0, 0.0], [120.0, 10.0], [60.0, 100.0]];
        let c = fit(&mk(&draw_poly(&tri, 4, &mut rng, 1.0, true))).expect("sparse triangle fits");
        assert!(matches!(c, Correction::Triangle(_)), "got {c:?}");
        // A small clean circle.
        let c = fit(&mk(&circle(50.0, 50.0, 12.0, 32, &mut rng, 0.5))).expect("small circle fits");
        assert!(matches!(c, Correction::Ellipse { .. }), "got {c:?}");
    }

    #[test]
    fn non_shapes_stay_raw() {
        let mut rng = Rng(0x0bad_c0de);
        // Retraced line (there and back): closed but degenerate.
        let mut raw: Vec<[f64; 2]> = (0..30).map(|i| [i as f64 * 5.0, 50.0]).collect();
        let back: Vec<[f64; 2]> = (0..30).rev().map(|i| [i as f64 * 5.0, 50.0]).collect();
        raw.extend(back);
        assert!(fit(&mk(&raw)).is_none(), "retraced line stays raw");
        // Open horseshoe: not a line, must smooth (never a bogus primitive).
        let mut hs: Vec<[f64; 2]> = (0..50)
            .map(|i| {
                let t = i as f64 / 50.0 * std::f64::consts::PI;
                [100.0 + 80.0 * t.cos(), 100.0 - 80.0 * t.sin()]
            })
            .collect();
        for (i, p) in hs.iter_mut().enumerate() {
            p[0] += (rng.next() - 0.5) * 4.0;
            p[1] += (rng.next() - 0.5) * 4.0;
            let _ = i;
        }
        let c = fit(&mk(&hs)).expect("horseshoe has a fallback");
        assert!(matches!(c, Correction::Smooth(_)), "got {c:?}");
        // Amoeba blob: closed, irregular, many corners — leave it alone.
        let blob: Vec<[f64; 2]> = (0..72)
            .map(|i| {
                let t = i as f64 / 72.0 * std::f64::consts::TAU;
                let r = 70.0 + 25.0 * (3.0 * t).sin() + 12.0 * (7.0 * t + 1.0).cos();
                [150.0 + r * t.cos(), 150.0 + r * t.sin()]
            })
            .collect();
        assert!(fit(&mk(&blob)).is_none(), "amoeba stays raw");
        // Thin but genuine ellipse: an eccentric fit is still a fit.
        let slash: Vec<[f64; 2]> = (0..48)
            .map(|i| {
                let t = i as f64 / 48.0 * std::f64::consts::TAU;
                [100.0 + 90.0 * t.cos(), 100.0 + 12.0 * t.sin()]
            })
            .collect();
        let c = fit(&mk(&slash)).expect("thin ellipse fits");
        if let Correction::Ellipse { rx, ry, .. } = c {
            assert!(rx > 60.0 && ry < 25.0, "eccentric axes {rx} {ry}");
        } else {
            panic!("got {c:?}");
        }
    }

    #[test]
    fn hostile_point_soup_never_panics_and_stays_finite() {
        let mut rng = Rng(0xf00d);
        for round in 0..300 {
            let n = 2 + (rng.next() * 60.0) as usize;
            let raw: Vec<StrokePoint> = (0..n)
                .map(|i| {
                    let pick = rng.next();
                    let xy = if pick < 0.1 {
                        [f64::NAN, rng.range(-500.0, 500.0)]
                    } else if pick < 0.15 {
                        [f64::INFINITY, f64::NEG_INFINITY]
                    } else if pick < 0.25 {
                        [0.0, 0.0]
                    } else {
                        [rng.range(-500.0, 500.0), rng.range(-500.0, 500.0)]
                    };
                    StrokePoint {
                        x: xy[0],
                        y: xy[1],
                        pressure: if rng.next() < 0.1 { f32::NAN } else { rng.next() as f32 },
                        tilt_x: 0.0,
                        tilt_y: 0.0,
                        rotation: 0.0,
                        wheel: 0.0,
                        time: i as f64 * 8.0,
                    }
                })
                .collect();
            if let Some(c) = fit(&raw) {
                // Every emitted coordinate is finite and sane.
                for p in sample(&c, 64) {
                    assert!(p[0].is_finite() && p[1].is_finite(), "round {round}: {p:?}");
                    assert!(p[0].abs() <= 2000.0 && p[1].abs() <= 2000.0, "round {round}: {p:?}");
                }
                if let Correction::Ellipse { rx, ry, .. } = c {
                    assert!(rx > 0.0 && rx.is_finite() && ry > 0.0 && ry.is_finite(), "round {round}");
                }
                for sp in stroke_points(&c, &raw) {
                    assert!(sp.x.is_finite() && sp.y.is_finite(), "round {round}");
                    assert!(sp.pressure.is_finite(), "round {round}: pressure sanitized");
                    assert!((0.0..=1.0).contains(&sp.pressure), "round {round}: pressure clamped");
                }
            }
            let _ = round;
        }
    }

    #[test]
    fn randomized_shapes_classify_reliably() {
        // 40 random shapes per kind at hand-like noise: the classifier must be exact.
        let mut rng = Rng(0x00c0_ffee);
        let mut exact = 0;
        let mut total = 0;
        for kind in 0..4 {
            for _ in 0..40 {
                let cx = rng.range(50.0, 400.0);
                let cy = rng.range(50.0, 400.0);
                let size = rng.range(60.0, 160.0);
                let rot_deg = rng.range(0.0, 360.0);
                let wob = size * 0.03;
                let put = |p: [f64; 2]| {
                    let q = rot([p[0] * size, p[1] * size], rot_deg);
                    [cx + q[0], cy + q[1]]
                };
                let raw: Vec<[f64; 2]> = match kind {
                    0 => (0..30)
                        .map(|i| {
                            let t = i as f64 / 30.0;
                            let p = put([t * 2.0 - 1.0, 0.0]);
                            [p[0] + (rng.next() - 0.5) * 2.0 * wob, p[1] + (rng.next() - 0.5) * 2.0 * wob]
                        })
                        .collect(),
                    1 => circle(cx, cy, size / 2.0, 48, &mut rng, wob),
                    2 => {
                        let c = [[-0.7, -0.7], [0.7, -0.7], [0.7, 0.7], [-0.7, 0.7]];
                        draw_poly(&c.map(put), 12, &mut rng, wob, true)
                    }
                    _ => {
                        let c = [[0.0, -0.8], [0.75, 0.6], [-0.75, 0.6]];
                        draw_poly(&c.map(put), 14, &mut rng, wob, true)
                    }
                };
                total += 1;
                match (kind, fit(&mk(&raw))) {
                    (0, Some(Correction::Line { .. })) | (0, Some(Correction::Smooth(_))) => exact += 1,
                    (1, Some(Correction::Ellipse { .. })) => exact += 1,
                    (2, Some(Correction::Rect(_))) => exact += 1,
                    (3, Some(Correction::Triangle(_))) => exact += 1,
                    (k, got) => eprintln!("miss kind={k} size={size:.0} rot={rot_deg:.0}: {got:?}"),
                }
            }
        }
        assert!(exact >= total * 95 / 100, "exact {exact}/{total}");
    }

    #[test]
    fn sampling_bounds_and_degenerate_counts() {
        let line = Correction::Line { a: [0.0, 0.0], b: [10.0, 0.0] };
        assert_eq!(sample(&line, 0).len(), 2, "count clamps to 2");
        assert_eq!(sample(&line, 1).len(), 2);
        assert!(sample(&line, 1_000_000).len() <= 513, "count clamps high");
        let ell = Correction::Ellipse { center: [0.0, 0.0], rx: 5.0, ry: 5.0, rot: 0.0 };
        let pts = sample(&ell, 32);
        assert!((pts.len() as i32 - 33).abs() <= 1, "closed loop samples");
        // Frozen time and empty input degrade gracefully.
        let mut raw = mk(&[[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]]);
        for p in &mut raw {
            p.time = 42.0;
        }
        let out = stroke_points(&line, &raw);
        assert!(out.windows(2).all(|w| w[1].time >= w[0].time));
    }

    #[test]
    fn duplicated_and_gapped_input() {
        // Pen-repeat duplicates plus a teleport gap: still a line.
        let mut coords = vec![[5.0, 5.0]; 30];
        for i in 0..30 {
            coords.push([10.0 + i as f64 * 5.0, 10.0]);
        }
        coords.extend(vec![[160.0, 10.0]; 20]);
        let c = fit(&mk(&coords)).expect("dup-stuffed line fits");
        assert!(matches!(c, Correction::Line { .. }), "got {c:?}");
        // A 5000-point straight stroke fits (DP budget caps the work).
        let big: Vec<[f64; 2]> = (0..5000).map(|i| [i as f64 * 0.1, 3.0]).collect();
        let c = fit(&mk(&big)).expect("big line fits");
        assert!(matches!(c, Correction::Line { .. }), "got {c:?}");
    }
}

#[cfg(test)]
mod dbg_move_tests {
    use super::super::*;
    use super::*;

    #[test]
    fn dbg_moved_end_refits() {
        let mut raw: Vec<[f64; 2]> = vec![[10.0, 50.0]];
        for i in 1..=20 {
            raw.push([10.0 + i as f64 * 7.0, 50.0 + if i % 2 == 0 { 3.0 } else { -3.0 }]);
        }
        let mut pts: Vec<StrokePoint> = raw
            .iter()
            .enumerate()
            .map(|(i, c)| StrokePoint {
                x: c[0],
                y: c[1],
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                wheel: 0.0,
                time: i as f64 * 20.0,
            })
            .collect();
        eprintln!("trigger fit: {:?}", fit(&pts).map(|c| std::mem::discriminant(&c)));
        pts.last_mut().unwrap().x = 150.0;
        pts.last_mut().unwrap().y = 90.0;
        eprintln!("moved fit: {:?}", fit(&pts).map(|c| std::mem::discriminant(&c)));
        if let Some(c) = fit(&pts) {
            let sp = stroke_points(&c, &pts);
            eprintln!("n={} first={:?} last={:?}", sp.len(), (sp[0].x, sp[0].y), (sp[sp.len()-1].x, sp[sp.len()-1].y));
        }
    }
}

#[cfg(test)]
mod stretch_tests {
    use super::*;

    #[test]
    fn stretch_rebuilds_between_fixed_start_and_moved_end() {
        let s = [10.0, 50.0];
        let e = [150.0, 90.0];
        assert_eq!(stretch(&Correction::Line { a: [0.0, 0.0], b: [1.0, 1.0] }, s, e), Some(Correction::Line { a: s, b: e }));
        assert_eq!(
            stretch(&Correction::Rect([[0.0, 0.0], [9.0, 0.0], [9.0, 9.0], [0.0, 9.0]]), s, e),
            Some(Correction::Rect([[10.0, 50.0], [150.0, 50.0], [150.0, 90.0], [10.0, 90.0]]))
        );
        assert_eq!(
            stretch(&Correction::Ellipse { center: [0.0, 0.0], rx: 1.0, ry: 1.0, rot: 0.0 }, s, e),
            Some(Correction::Ellipse { center: [80.0, 70.0], rx: 70.0, ry: 20.0, rot: 0.0 })
        );
        // A degenerate span and non-finite input keep the old shape (None).
        assert_eq!(stretch(&Correction::Line { a: s, b: e }, s, [s[0] + 0.5, s[1]]), None);
        assert_eq!(stretch(&Correction::Line { a: s, b: e }, s, [f64::NAN, 0.0]), None);
        // Triangles affine-map, keeping their proportions.
        let tri = Correction::Triangle([[0.0, 0.0], [100.0, 0.0], [50.0, 100.0]]);
        if let Some(Correction::Triangle(t)) = stretch(&tri, [0.0, 0.0], [200.0, 100.0]) {
            assert_eq!(t[0], [0.0, 0.0]);
            assert_eq!(t[1], [200.0, 0.0]);
            assert_eq!(t[2], [100.0, 100.0]);
        } else {
            panic!("triangle stretches");
        }
        // Smooth has no primitive: the caller refits instead.
        assert_eq!(stretch(&Correction::Smooth(vec![s, e]), s, e), None);
    }
}
