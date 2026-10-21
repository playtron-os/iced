//! Load and operate on images.
#[cfg(feature = "image")]
use crate::core::Bytes;

use crate::core::Rectangle;
use crate::core::image;
use crate::core::svg;

/// A raster or vector image.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq)]
pub enum Image {
    /// A raster image.
    Raster {
        image: image::Image,
        bounds: Rectangle,
        clip_bounds: Rectangle,
    },

    /// A vector image.
    Vector {
        svg: svg::Svg,
        bounds: Rectangle,
        clip_bounds: Rectangle,
    },
}

impl Image {
    /// Scales the image's opacity, as an opacity group does to images drawn
    /// inside it.
    pub fn scale_opacity(&mut self, opacity: f32) {
        match self {
            Image::Raster { image, .. } => image.opacity *= opacity,
            Image::Vector { svg, .. } => svg.opacity *= opacity,
        }
    }

    /// Returns the bounds of the [`Image`].
    pub fn bounds(&self) -> Rectangle {
        match self {
            Image::Raster { image, bounds, .. } => bounds.rotate(image.rotation),
            Image::Vector { svg, bounds, .. } => bounds.rotate(svg.rotation),
        }
    }
}

/// An image buffer.
#[cfg(feature = "image")]
pub type Buffer = ::image::ImageBuffer<::image::Rgba<u8>, Bytes>;

/// The bytes past which an image is large: too big for wgpu to upload in the
/// frame that draws it, and reduced by every renderer when drawn smaller.
pub const LARGE: usize = 2 * 1024 * 1024;

/// How many times to halve an image of `size` drawn over `drawn` device
/// pixels: as often as the result still covers them. Only a [`LARGE`] image
/// is halved, so a small one keeps its own pixels.
pub fn reduction(size: crate::core::Size<u32>, drawn: crate::core::Size<f32>) -> u32 {
    if size.width as usize * size.height as usize * 4 < LARGE {
        return 0;
    }

    let ratio =
        (size.width as f32 / drawn.width.max(1.0)).min(size.height as f32 / drawn.height.max(1.0));

    if ratio < 2.0 {
        return 0;
    }

    ratio.log2().floor() as u32
}

/// Halves `image` `levels` times. Each pixel averages a square of the
/// source, weighted by alpha so transparent pixels do not darken the edges.
#[cfg(feature = "image")]
pub fn reduce(image: &Buffer, levels: u32) -> Buffer {
    let (width, height) = image.dimensions();
    let shift = levels.min(31);
    let block = 1u32 << shift;
    let (out_width, out_height) = (width.div_ceil(block), height.div_ceil(block));
    let source = image.as_raw();
    let stride = width as usize * 4;

    let mut out = Vec::with_capacity(out_width as usize * out_height as usize * 4);
    // Per output column: alpha, and red, green and blue times alpha.
    let mut sums = vec![[0u64; 4]; out_width as usize];

    for out_y in 0..out_height {
        let top = out_y * block;
        let rows = block.min(height - top);

        sums.iter_mut().for_each(|sum| *sum = [0; 4]);

        for y in top..top + rows {
            let row = &source[y as usize * stride..][..stride];

            for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
                let alpha = u64::from(pixel[3]);
                let sum = &mut sums[x >> shift];

                sum[0] += alpha;
                sum[1] += u64::from(pixel[0]) * alpha;
                sum[2] += u64::from(pixel[1]) * alpha;
                sum[3] += u64::from(pixel[2]) * alpha;
            }
        }

        for (out_x, sum) in sums.iter().enumerate() {
            let columns = block.min(width - out_x as u32 * block);
            let count = u64::from(columns) * u64::from(rows);
            let color =
                |channel: u64| (channel + sum[0] / 2).checked_div(sum[0]).unwrap_or(0) as u8;

            out.extend_from_slice(&[
                color(sum[1]),
                color(sum[2]),
                color(sum[3]),
                ((sum[0] + count / 2) / count) as u8,
            ]);
        }
    }

    ::image::ImageBuffer::from_raw(out_width, out_height, Bytes::from(out))
        .expect("the reduced image fills its buffer")
}

#[cfg(feature = "image")]
/// Tries to load an image by its [`Handle`].
///
/// [`Handle`]: image::Handle
pub fn load(handle: &image::Handle) -> Result<Buffer, image::Error> {
    use bitflags::bitflags;

    bitflags! {
        struct Operation: u8 {
            const FLIP_HORIZONTALLY = 0b1;
            const ROTATE_180 = 0b10;
            const FLIP_VERTICALLY= 0b100;
            const ROTATE_90 = 0b1000;
            const ROTATE_270 = 0b10000;
        }
    }

    impl Operation {
        // Meaning of the returned value is described e.g. at:
        // https://magnushoff.com/articles/jpeg-orientation/
        fn from_exif<R>(reader: &mut R) -> Result<Self, exif::Error>
        where
            R: std::io::BufRead + std::io::Seek,
        {
            let exif = exif::Reader::new().read_from_container(reader)?;

            Ok(exif
                .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|field| field.value.get_uint(0))
                .and_then(|value| u8::try_from(value).ok())
                .map(|value| match value {
                    1 => Operation::empty(),
                    2 => Operation::FLIP_HORIZONTALLY,
                    3 => Operation::ROTATE_180,
                    4 => Operation::FLIP_VERTICALLY,
                    5 => Operation::ROTATE_90 | Operation::FLIP_HORIZONTALLY,
                    6 => Operation::ROTATE_90,
                    7 => Operation::ROTATE_90 | Operation::FLIP_VERTICALLY,
                    8 => Operation::ROTATE_270,
                    _ => Operation::empty(),
                })
                .unwrap_or_else(Self::empty))
        }

        fn perform(self, mut image: ::image::DynamicImage) -> ::image::DynamicImage {
            use ::image::imageops;

            if self.contains(Operation::ROTATE_90) {
                image = imageops::rotate90(&image).into();
            }

            if self.contains(Self::ROTATE_180) {
                imageops::rotate180_in_place(&mut image);
            }

            if self.contains(Operation::ROTATE_270) {
                image = imageops::rotate270(&image).into();
            }

            if self.contains(Self::FLIP_VERTICALLY) {
                imageops::flip_vertical_in_place(&mut image);
            }

            if self.contains(Self::FLIP_HORIZONTALLY) {
                imageops::flip_horizontal_in_place(&mut image);
            }

            image
        }
    }

    let (width, height, pixels) = match handle {
        image::Handle::Path(_, path) => {
            let image = open_image(path).map_err(to_error)?;

            let operation = std::fs::File::open(path)
                .ok()
                .map(std::io::BufReader::new)
                .and_then(|mut reader| Operation::from_exif(&mut reader).ok())
                .unwrap_or_else(Operation::empty);

            let rgba = operation.perform(image).into_rgba8();

            (rgba.width(), rgba.height(), Bytes::from(rgba.into_raw()))
        }
        image::Handle::Bytes(_, bytes) => {
            let image = load_from_memory(bytes).map_err(to_error)?;

            let operation = Operation::from_exif(&mut std::io::Cursor::new(bytes))
                .ok()
                .unwrap_or_else(Operation::empty);

            let rgba = operation.perform(image).into_rgba8();

            (rgba.width(), rgba.height(), Bytes::from(rgba.into_raw()))
        }
        image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } => (*width, *height, pixels.clone()),
    };

    if let Some(image) = ::image::ImageBuffer::from_raw(width, height, pixels) {
        Ok(image)
    } else {
        Err(to_error(::image::error::ImageError::Limits(
            ::image::error::LimitError::from_kind(::image::error::LimitErrorKind::DimensionError),
        )))
    }
}

#[cfg(feature = "image")]
fn to_error(error: ::image::ImageError) -> image::Error {
    use std::sync::Arc;

    match error {
        ::image::ImageError::IoError(error) => image::Error::Inaccessible(Arc::new(error)),
        error => image::Error::Invalid(Arc::new(error)),
    }
}

/// JPEG XL naked codestream magic bytes.
#[cfg(feature = "image")]
const JXL_CODESTREAM_MAGIC: [u8; 2] = [0xFF, 0x0A];

/// JPEG XL ISO BMFF container magic bytes.
#[cfg(feature = "image")]
const JXL_CONTAINER_MAGIC: [u8; 8] = [0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20];

/// Open an image from a file path, with JPEG XL support.
/// Detects JXL by magic bytes (not just extension) for robustness.
#[cfg(feature = "image")]
fn open_image(path: &std::path::Path) -> Result<::image::DynamicImage, ::image::ImageError> {
    // Read first 12 bytes to detect JXL by magic (handles extensionless/misnamed files)
    let is_jxl = std::fs::File::open(path)
        .and_then(|mut f| {
            use std::io::Read;
            let mut header = [0u8; 12];
            f.read_exact(&mut header).map(|_| header)
        })
        .map(|h| h.starts_with(&JXL_CODESTREAM_MAGIC) || h.starts_with(&JXL_CONTAINER_MAGIC))
        .unwrap_or_else(|_| {
            // Fallback to extension if we can't read the header
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jxl"))
        });

    if is_jxl {
        return decode_jxl_file(path);
    }

    ::image::open(path)
}

/// Load an image from memory bytes, with JPEG XL support.
#[cfg(feature = "image")]
fn load_from_memory(bytes: &[u8]) -> Result<::image::DynamicImage, ::image::ImageError> {
    let is_jxl =
        bytes.starts_with(&JXL_CONTAINER_MAGIC) || bytes.starts_with(&JXL_CODESTREAM_MAGIC);

    if is_jxl {
        return decode_jxl_bytes(bytes);
    }

    ::image::load_from_memory(bytes)
}

/// Decode a JPEG XL file from disk via jxl-oxide.
#[cfg(feature = "image")]
fn decode_jxl_file(path: &std::path::Path) -> Result<::image::DynamicImage, ::image::ImageError> {
    let file = std::fs::File::open(path).map_err(::image::ImageError::IoError)?;
    let reader = std::io::BufReader::new(file);
    decode_jxl(reader)
}

/// Decode JPEG XL from memory bytes via jxl-oxide.
#[cfg(feature = "image")]
fn decode_jxl_bytes(bytes: &[u8]) -> Result<::image::DynamicImage, ::image::ImageError> {
    decode_jxl(std::io::Cursor::new(bytes))
}

/// Shared JXL decode logic — accepts any `BufRead + Seek` reader.
#[cfg(feature = "image")]
fn decode_jxl<R: std::io::BufRead + std::io::Seek>(
    reader: R,
) -> Result<::image::DynamicImage, ::image::ImageError> {
    use jxl_oxide::integration::JxlDecoder;

    let decoder = JxlDecoder::new(reader).map_err(|e| {
        ::image::ImageError::Decoding(::image::error::DecodingError::new(
            ::image::error::ImageFormatHint::Name("jxl".to_string()),
            e,
        ))
    })?;

    ::image::DynamicImage::from_decoder(decoder)
}

#[cfg(all(test, feature = "image"))]
mod tests {
    use super::*;
    use crate::core::Size;

    fn buffer(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> Buffer {
        ::image::ImageBuffer::from_raw(width, height, Bytes::from(pixels.concat())).unwrap()
    }

    #[test]
    fn a_large_image_halves_as_often_as_it_still_covers_the_draw() {
        let size = Size::new(2400, 3600);

        assert_eq!(reduction(size, Size::new(64.0, 96.0)), 5);
        assert_eq!(reduction(size, Size::new(1200.0, 1800.0)), 1);
        assert_eq!(reduction(size, Size::new(1201.0, 1800.0)), 0);
        assert_eq!(reduction(size, Size::new(2400.0, 3600.0)), 0);
    }

    #[test]
    fn a_small_image_keeps_its_own_pixels() {
        assert_eq!(reduction(Size::new(512, 512), Size::new(8.0, 8.0)), 0);
    }

    #[test]
    fn reducing_averages_by_coverage_and_keeps_partial_edges() {
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let clear = [0, 0, 0, 0];
        // 3×2: a red and blue pair, a red pixel beside a clear one, and an
        // odd column.
        let image = buffer(3, 2, vec![red, blue, red, red, clear, blue]);

        let reduced = reduce(&image, 1);

        assert_eq!(reduced.dimensions(), (2, 1));
        // Three opaque pixels of four: red twice, blue once, a quarter clear.
        assert_eq!(reduced.get_pixel(0, 0).0, [170, 0, 85, 191]);
        // The odd column averages the two pixels it has.
        assert_eq!(reduced.get_pixel(1, 0).0, [128, 0, 128, 255]);
    }
}
