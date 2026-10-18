//! Text recognition by Windows (`Windows.Media.Ocr`), the recogniser built into Windows 10 and
//! 11. It reads every language Windows has the optical character recognition feature for,
//! accents included (Portuguese, French, German…), with nothing to ship: the language packs come
//! with Windows or from its Settings.

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

use crate::{Line, OcrError, Word};

/// Largest image this module hands to Windows when it can't ask: Windows has accepted at least
/// this since Windows 10 (its `MaxImageDimension` is the real limit, read at run time).
const FALLBACK_MAX_DIMENSION: u32 = 2600;

/// Cap on the pixels this accepts (RGBA bytes = 4 × this): a page at 600 dpi on A3 is about
/// 70 Mpx; anything far beyond is not a page.
const MAX_PIXELS: u64 = 200_000_000;

fn err(e: windows::core::Error) -> OcrError {
    OcrError::Recognize(e.message())
}

/// A Windows recogniser for one language.
pub struct WinOcr {
    engine: OcrEngine,
}

impl WinOcr {
    /// The recogniser for `tag` (a BCP-47 tag such as "pt-BR" or "en-US"), or
    /// [`OcrError::LanguageNotInstalled`] when Windows has no OCR for it.
    pub fn new(tag: &str) -> Result<WinOcr, OcrError> {
        let language = Language::CreateLanguage(&HSTRING::from(tag)).map_err(|_| OcrError::LanguageNotInstalled)?;
        if !OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
            return Err(OcrError::LanguageNotInstalled);
        }
        let engine = OcrEngine::TryCreateFromLanguage(&language).map_err(|_| OcrError::LanguageNotInstalled)?;
        Ok(WinOcr { engine })
    }

    /// Recognise the text in an RGBA image, `width` × `height` pixels.
    pub fn recognize(&self, pixels: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        let n = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || n > MAX_PIXELS || pixels.len() as u64 != n * 4 {
            return Err(OcrError::EmptyImage);
        }
        // Windows refuses images larger than its limit: shrink by a whole factor, reading each
        // block's average, and scale the word boxes back.
        let max = OcrEngine::MaxImageDimension().unwrap_or(FALLBACK_MAX_DIMENSION).max(1);
        let factor = width.max(height).div_ceil(max).max(1);
        let (w, h) = ((width / factor).max(1), (height / factor).max(1));
        let mut bgra = Vec::with_capacity((w as usize) * (h as usize) * 4);
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut count) = ([0u32; 3], 0u32);
                for dy in 0..factor {
                    for dx in 0..factor {
                        let (sx, sy) = (x * factor + dx, y * factor + dy);
                        if sx >= width || sy >= height {
                            continue;
                        }
                        let at = ((sy as usize) * (width as usize) + sx as usize) * 4;
                        if let Some(p) = pixels.get(at..at + 4) {
                            // Flatten transparency onto white, as the page is shown.
                            let a = u32::from(p[3]);
                            for (s, c) in sum.iter_mut().zip(&p[..3]) {
                                *s += (u32::from(*c) * a + 255 * (255 - a)) / 255;
                            }
                            count += 1;
                        }
                    }
                }
                let c = count.max(1);
                bgra.extend_from_slice(&[(sum[2] / c) as u8, (sum[1] / c) as u8, (sum[0] / c) as u8, 255]);
            }
        }
        let writer = DataWriter::new().map_err(err)?;
        writer.WriteBytes(&bgra).map_err(err)?;
        let buffer = writer.DetachBuffer().map_err(err)?;
        let w_i32 = i32::try_from(w).map_err(|_| OcrError::EmptyImage)?;
        let h_i32 = i32::try_from(h).map_err(|_| OcrError::EmptyImage)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w_i32, h_i32).map_err(err)?;
        let result = self.engine.RecognizeAsync(&bitmap).map_err(err)?.join().map_err(err)?;
        let f = factor as f32;
        let mut out = Vec::new();
        for line in result.Lines().map_err(err)? {
            let mut words = Vec::new();
            for word in line.Words().map_err(err)? {
                let text = word.Text().map_err(err)?.to_string();
                if text.trim().is_empty() {
                    continue;
                }
                let r = word.BoundingRect().map_err(err)?;
                words.push(Word { text, rect: [r.X * f, r.Y * f, (r.X + r.Width) * f, (r.Y + r.Height) * f] });
            }
            if !words.is_empty() {
                out.push(Line { words });
            }
        }
        Ok(out)
    }
}
