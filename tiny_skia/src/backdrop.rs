//! Ordered, opt-in filtering of the pixels already behind a widget.

use crate::core::{Rectangle, renderer::BackdropFilter};

type Pixel = [f32; 4];

pub fn draw(
    pixels: &mut tiny_skia::PixmapMut<'_>,
    filter: &BackdropFilter,
    opacity: f32,
    scale: f32,
    clip: Rectangle,
    clip_mask: &tiny_skia::Mask,
) {
    let bounds = filter.bounds * scale;
    if !opacity.is_finite()
        || opacity <= 0.0
        || !filter.radius.is_finite()
        || !filter.saturation.is_finite()
        || !valid(bounds)
    {
        return;
    }
    let width = pixels.width() as usize;
    let height = pixels.height() as usize;
    let screen = Rectangle {
        x: 0.0,
        y: 0.0,
        width: width as f32,
        height: height as f32,
    };
    let Some(region) = bounds
        .intersection(&clip)
        .and_then(|area| area.intersection(&screen))
    else {
        return;
    };
    let left = region.x.floor().max(0.0) as usize;
    let top = region.y.floor().max(0.0) as usize;
    let right = (region.x + region.width).ceil().min(width as f32) as usize;
    let bottom = (region.y + region.height).ceil().min(height as f32) as usize;
    let Some(mut mask) = tiny_skia::Mask::new((right - left) as u32, (bottom - top) as u32) else {
        return;
    };
    let radii = filter.border_radius.map(|radius| {
        (radius * scale)
            .max(0.0)
            .min(bounds.width / 2.0)
            .min(bounds.height / 2.0)
    });
    mask.fill_path(
        &crate::engine::rounded_rectangle(bounds, radii),
        tiny_skia::FillRule::Winding,
        true,
        tiny_skia::Transform::from_translate(-(left as f32), -(top as f32)),
    );

    let sigma = f64::from(filter.radius.max(0.0)) * f64::from(scale);
    let radius = box_radius(sigma);
    let padding =
        ((3.0 * sigma).ceil().min((usize::MAX / 4) as f64) as usize).max(radius.saturating_mul(3));
    let sample_left = left.saturating_sub(padding);
    let sample_top = top.saturating_sub(padding);
    let sample_right = right.saturating_add(padding).min(width);
    let sample_bottom = bottom.saturating_add(padding).min(height);
    let sample_width = sample_right - sample_left;
    let sample_height = sample_bottom - sample_top;
    let blurred = if radius == 0 {
        None
    } else {
        let Some(mut source) =
            snapshot(pixels, sample_left, sample_top, sample_width, sample_height)
        else {
            return;
        };
        let mut scratch = Vec::new();
        if scratch.try_reserve_exact(source.len()).is_err() {
            return;
        }
        scratch.resize(source.len(), [0.0; 4]);
        for _ in 0..3 {
            box_pass(
                &source,
                &mut scratch,
                sample_width,
                sample_height,
                radius,
                true,
            );
            box_pass(
                &scratch,
                &mut source,
                sample_width,
                sample_height,
                radius,
                false,
            );
        }
        Some(source)
    };

    let opacity = opacity.clamp(0.0, 1.0);
    let saturation = filter.saturation.max(0.0);
    for y in top..bottom {
        for x in left..right {
            let index = y * width + x;
            let coverage = f32::from(mask.data()[(y - top) * (right - left) + x - left])
                * f32::from(clip_mask.data().get(index).copied().unwrap_or(0))
                / (255.0 * 255.0);
            let weight = coverage * opacity * fade(filter, bounds, x as f32 + 0.5, y as f32 + 0.5);
            if weight <= 0.0 {
                continue;
            }
            let original = channels(pixels.as_ref().pixels()[index]);
            let mut filtered = blurred.as_ref().map_or(original, |blurred| {
                blurred[(y - sample_top) * sample_width + x - sample_left]
            });
            if (saturation - 1.0).abs() > 0.001 {
                // tiny-skia stores BGR here for softbuffer; CSS works in encoded sRGB.
                let luma = filtered[0] * 0.072 + filtered[1] * 0.715 + filtered[2] * 0.213;
                let alpha = filtered[3];
                for channel in &mut filtered[..3] {
                    *channel = (luma + (*channel - luma) * saturation).clamp(0.0, alpha);
                }
            }
            let alpha = (original[3] + (filtered[3] - original[3]) * weight).clamp(0.0, 255.0);
            let color: [u8; 3] = std::array::from_fn(|channel| {
                (original[channel] + (filtered[channel] - original[channel]) * weight)
                    .clamp(0.0, alpha)
                    .round() as u8
            });
            pixels.pixels_mut()[index] = tiny_skia::PremultipliedColorU8::from_rgba(
                color[0],
                color[1],
                color[2],
                alpha.round() as u8,
            )
            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
    }
}

fn valid(bounds: Rectangle) -> bool {
    bounds.width > 0.0
        && bounds.height > 0.0
        && [
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            bounds.x + bounds.width,
            bounds.y + bounds.height,
        ]
        .into_iter()
        .all(f32::is_finite)
}

fn channels(pixel: tiny_skia::PremultipliedColorU8) -> Pixel {
    [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()].map(f32::from)
}

fn snapshot(
    pixels: &tiny_skia::PixmapMut<'_>,
    left: usize,
    top: usize,
    width: usize,
    height: usize,
) -> Option<Vec<Pixel>> {
    let mut source = Vec::new();
    source.try_reserve_exact(width.checked_mul(height)?).ok()?;
    for row in pixels
        .as_ref()
        .pixels()
        .chunks_exact(pixels.width() as usize)
        .skip(top)
        .take(height)
    {
        source.extend(row[left..left + width].iter().copied().map(channels));
    }
    Some(source)
}

// Three box passes use the same W3C box width as the GPU blur shader.
fn box_radius(sigma: f64) -> usize {
    let width = (sigma.max(1.0) * 1.8799 + 0.5).floor();
    (width.min((usize::MAX / 8) as f64) as usize).saturating_sub(1) / 2
}

fn box_pass(
    source: &[Pixel],
    target: &mut [Pixel],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) {
    let (lines, length, step) = if horizontal {
        (height, width, 1)
    } else {
        (width, height, width)
    };
    let divisor = 2.0 * radius as f64 + 1.0;
    for line in 0..lines {
        let base = if horizontal { line * width } else { line };
        let first = source[base];
        let last = source[base + (length - 1) * step];
        let mut sum = first.map(|value| f64::from(value) * (radius as f64 + 1.0));
        for position in 1..=radius.min(length - 1) {
            for (channel, value) in sum.iter_mut().enumerate() {
                *value += f64::from(source[base + position * step][channel]);
            }
        }
        if radius >= length {
            for (channel, value) in sum.iter_mut().enumerate() {
                *value += f64::from(last[channel]) * (radius - length + 1) as f64;
            }
        }
        for position in 0..length {
            target[base + position * step] = sum.map(|value| (value / divisor) as f32);
            let leaving = position.saturating_sub(radius);
            let entering = position
                .saturating_add(radius)
                .saturating_add(1)
                .min(length - 1);
            for (channel, value) in sum.iter_mut().enumerate() {
                *value += f64::from(source[base + entering * step][channel])
                    - f64::from(source[base + leaving * step][channel]);
            }
        }
    }
}

fn fade(filter: &BackdropFilter, bounds: Rectangle, x: f32, y: f32) -> f32 {
    let start = filter.fade_start.clamp(0.0, 1.0);
    let end = filter.fade_end.clamp(0.0, 1.0);
    if !start.is_finite() || !end.is_finite() || start >= end {
        return 1.0;
    }
    let x = (x - bounds.x) / bounds.width;
    let y = (y - bounds.y) / bounds.height;
    match filter.fade_direction.min(5) {
        4 => (y / end).min((1.0 - y) / end).clamp(0.0, 1.0),
        5 => (x / end).min((1.0 - x) / end).clamp(0.0, 1.0),
        direction => {
            let position = if direction < 2 { y } else { x };
            let progress = ((position - start) / (end - start)).clamp(0.0, 1.0);
            if matches!(direction, 1 | 3) {
                progress
            } else {
                1.0 - progress
            }
        }
    }
}

#[cfg(test)]
#[path = "backdrop_tests.rs"]
mod tests;
