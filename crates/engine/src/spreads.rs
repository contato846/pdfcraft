//! Pages ▸ Split book spreads: a book scanned open, two pages to a sheet, becomes one page per
//! book page, the left one first.
//!
//! Nothing is cut out of the scan. Each spread page is duplicated and each copy's media and crop
//! boxes keep one half: the picture stays as it was scanned, at its own resolution, and the split
//! is instant and undoable. Only pages wider than tall are split (a cover or a page scanned on its
//! own stays as it is).
//!
//! Where to split is found on a small rendering of the page: the binding is the band near the
//! middle that no line of text crosses, whether it shows as the shadow of a flatbed scan or as the
//! light gap of a camera scan. Text makes the rows change from light to dark many times; the
//! binding is smooth. When nothing stands out the page is split in the middle.

use pdfcraft_render::{PageInfo, PageRenderer, RenderConfig, RenderRequest};

use crate::{DocId, Edit, EditError, Session};

/// The binding is looked for in this band of the page width (from the left edge, as displayed).
const BAND: (f32, f32) = (0.35, 0.65);

/// A page is a spread when its displayed width is at least this times its height.
pub const MIN_SPREAD_RATIO: f32 = 1.15;

/// Pixels across the rendering the binding is looked for in.
const PROBE_WIDTH: f32 = 480.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAt {
    /// Where the binding is found.
    Binding,
    /// The middle of the page.
    Middle,
}

/// One spread to split: the page and where, as a fraction of its displayed width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub page: usize,
    pub at: f32,
}

/// Where the binding of a spread is, as a fraction of the image width, in a grey image
/// (`width` × `height`, one byte a pixel, row by row).
pub fn find_binding(grey: &[u8], width: usize, height: usize) -> f32 {
    if width < 8 || height < 8 || grey.len() < width.saturating_mul(height) {
        return 0.5;
    }
    // The middle rows: page edges, the scanner lid and the head and foot shadows stay out.
    let (top, bottom) = (height / 10, height - height / 10);
    let px = |x: usize, y: usize| i32::from(grey.get(y * width + x).copied().unwrap_or(255));
    // How busy each column is: the vertical changes of light down it.
    let busy: Vec<f32> = (0..width)
        .map(|x| {
            let mut sum = 0u64;
            for y in top + 1..bottom {
                sum += (px(x, y) - px(x, y - 1)).unsigned_abs() as u64;
            }
            sum as f32 / (bottom - top).max(1) as f32
        })
        .collect();
    let (from, to) = ((width as f32 * BAND.0) as usize, (width as f32 * BAND.1) as usize);
    let reach = (width / 80).max(2);
    let smooth = |x: usize| {
        let (a, b) = (x.saturating_sub(reach), (x + reach + 1).min(width));
        busy.get(a..b).map_or(0.0, |s| s.iter().sum::<f32>() / s.len().max(1) as f32)
    };
    let scores: Vec<(usize, f32)> = (from..to).map(|x| (x, smooth(x))).collect();
    let (Some(least), Some(most)) = (scores.iter().map(|s| s.1).reduce(f32::min), scores.iter().map(|s| s.1).reduce(f32::max)) else {
        return 0.5;
    };
    // A band as busy all across (a picture, a blank sheet): no binding to find.
    if most - least < 0.5 || most < least * 1.5 {
        return 0.5;
    }
    // The quietest stretch: the columns within a small margin of the quietest, and its middle
    // (a wide gap is split down its centre).
    let quiet = least + (most - least) * 0.08;
    let centre = width as f32 / 2.0;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &(x, s) in &scores {
        match runs.last_mut() {
            Some(r) if s <= quiet && r.1 + 1 == x => r.1 = x,
            _ if s <= quiet => runs.push((x, x)),
            _ => {}
        }
    }
    // A binding has text on both sides: a quiet stretch reaching the edge of the band is a blank
    // page (a verso left empty before a chapter) or a margin, not the binding. Several quiet
    // stretches (an illustration): the one nearest the centre.
    let (first, last) = (scores.first().map_or(0, |s| s.0), scores.last().map_or(0, |s| s.0));
    runs.into_iter()
        .filter(|(a, b)| *a > first && *b < last)
        .map(|(a, b)| (a + b) as f32 / 2.0)
        .min_by(|a, b| (a - centre).abs().total_cmp(&(b - centre).abs()))
        .map_or(0.5, |x| (x + 0.5) / width as f32)
}

/// Whether a page is a spread (wider than tall, as displayed).
pub fn is_spread(info: &PageInfo) -> bool {
    info.height > 0.0 && info.width >= info.height * MIN_SPREAD_RATIO
}

/// Everything finding the bindings needs, detached from the session (the UI runs it on a worker
/// thread).
pub struct SpreadJob {
    bytes: std::sync::Arc<Vec<u8>>,
    password: Option<String>,
    infos: Vec<PageInfo>,
    pub pages: Vec<usize>,
    pub at: SplitAt,
}

impl SpreadJob {
    /// Where to split each spread. `progress(done, total)` is called before each page; returning
    /// `false` stops (the pages looked at so far are returned).
    pub fn run(self, mut progress: impl FnMut(usize, usize) -> bool) -> Vec<Spread> {
        let config = RenderConfig { password: self.password.as_deref().map(std::sync::Arc::from), ..Default::default() };
        let mut r = PageRenderer::new(self.bytes.clone(), config);
        let total = self.pages.len();
        let mut out = Vec::new();
        for (done, &page) in self.pages.iter().enumerate() {
            if !progress(done, total) {
                break;
            }
            let Some(info) = self.infos.get(page) else { continue };
            let at = match self.at {
                SplitAt::Middle => 0.5,
                SplitAt::Binding => {
                    let scale = PROBE_WIDTH / info.width.max(1.0);
                    let shot = r.render(RenderRequest { page, scale, ..Default::default() });
                    let (w, h) = (shot.width as usize, shot.height as usize);
                    if shot.error.is_some() || w == 0 || h == 0 {
                        0.5
                    } else {
                        let grey: Vec<u8> =
                            shot.rgba.chunks_exact(4).map(|p| ((u32::from(p[0]) * 3 + u32::from(p[1]) * 6 + u32::from(p[2])) / 10) as u8).collect();
                        find_binding(&grey, w, h)
                    }
                }
            };
            out.push(Spread { page, at });
        }
        progress(total, total);
        out
    }
}

/// The user-space rectangle shown between `from` and `to` (fractions of the displayed width).
fn half(info: &PageInfo, from: f32, to: f32) -> [f64; 4] {
    let (w, h) = (info.width, info.height);
    let a = info.view_to_user(w * from, 0.0);
    let b = info.view_to_user(w * to, h);
    [f64::from(a[0].min(b[0])), f64::from(a[1].min(b[1])), f64::from(a[0].max(b[0])), f64::from(a[1].max(b[1]))]
}

impl Session {
    /// The spreads among `pages` (0-based; empty = all) of document `id`, ready to look for
    /// their bindings.
    pub fn spread_job(&self, id: DocId, pages: &[usize], at: SplitAt) -> Option<SpreadJob> {
        let doc = self.get(id)?;
        let all = doc.info.pages.len();
        let pages: Vec<usize> = if pages.is_empty() { (0..all).collect() } else { pages.iter().copied().filter(|p| *p < all).collect() };
        let pages = pages.into_iter().filter(|p| doc.info.pages.get(*p).is_some_and(is_spread)).collect();
        Some(SpreadJob { bytes: doc.bytes.clone(), password: doc.password.clone(), infos: doc.info.pages.clone(), pages, at })
    }

    /// Split `spreads` as one undoable step. Returns how many were split.
    pub fn apply_spreads(&mut self, id: DocId, spreads: &[Spread]) -> Result<usize, EditError> {
        let doc = self.get(id).ok_or_else(|| EditError::Invalid("no such document".into()))?;
        let mut edits: Vec<Edit> = spreads
            .iter()
            .filter_map(|s| {
                let info = doc.info.pages.get(s.page)?;
                let at = if s.at.is_finite() { s.at.clamp(0.05, 0.95) } else { 0.5 };
                Some(Edit::SplitSpread { page: s.page, left: half(info, 0.0, at), right: half(info, at, 1.0) })
            })
            .collect();
        // From the last page back, so each split leaves the earlier pages where they were.
        edits.sort_by_key(|e| std::cmp::Reverse(if let Edit::SplitSpread { page, .. } = e { *page } else { 0 }));
        edits.dedup_by_key(|e| if let Edit::SplitSpread { page, .. } = e { *page } else { usize::MAX });
        let n = edits.len();
        if n > 0 {
            self.apply(id, Edit::Batch { label: "Split book spreads".into(), edits })?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey spread `w` × `h`: lines of "text" (dark dashes) across two blocks, `[a, b)` and
    /// `[c, d)` of the width, and an optional shadow band.
    fn spread(w: usize, h: usize, blocks: [(f32, f32); 2], shadow: Option<(f32, f32)>) -> Vec<u8> {
        let mut g = vec![245u8; w * h];
        for y in 0..h {
            let line = (y / 6) % 2 == 0 && y > h / 8 && y < h - h / 8;
            for x in 0..w {
                let f = x as f32 / w as f32;
                let inked = line && blocks.iter().any(|(a, b)| f >= *a && f < *b) && (x / 3) % 4 != 0;
                if inked {
                    g[y * w + x] = 30;
                }
                if let Some((a, b)) = shadow
                    && f >= a
                    && f < b
                {
                    g[y * w + x] = g[y * w + x].min(120);
                }
            }
        }
        g
    }

    #[test]
    fn the_binding_is_the_band_no_text_crosses() {
        let (w, h) = (480, 320);
        let g = spread(w, h, [(0.06, 0.42), (0.50, 0.94)], None);
        let at = find_binding(&g, w, h);
        assert!((at - 0.46).abs() < 0.02, "{at}");
        // Off centre, with the shadow of a flatbed scan in the gap.
        let g = spread(w, h, [(0.05, 0.50), (0.58, 0.95)], Some((0.51, 0.57)));
        let at = find_binding(&g, w, h);
        assert!((at - 0.54).abs() < 0.02, "{at}");
    }

    /// A PDF: a portrait cover (200 × 300), then a 600 × 400 spread whose text blocks leave the
    /// binding at x = 270 (0.45 of the width).
    fn book() -> Vec<u8> {
        let mut text = String::from("0 g ");
        for line in 0..24 {
            let y = 360 - line * 13;
            for (a, b) in [(30, 250), (290, 570)] {
                let mut x = a;
                while x < b {
                    text.push_str(&format!("{x} {y} 14 6 re "));
                    x += 18;
                }
            }
        }
        text.push('f');
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 400] /Contents 5 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    #[test]
    fn a_spread_becomes_two_pages_cut_at_its_binding() {
        let mut s = Session::new().with_clock(|| 1_700_000_000);
        let id = s.open("book.pdf", None, std::sync::Arc::new(book()), None).expect("opens");
        let job = s.spread_job(id, &[], SplitAt::Binding).unwrap();
        assert_eq!(job.pages, [1], "only the spread; the cover stands as it is");
        let found = job.run(|_, _| true);
        assert_eq!(found.len(), 1);
        assert!((found[0].at - 0.45).abs() < 0.02, "binding at {}", found[0].at);
        assert_eq!(s.apply_spreads(id, &found).unwrap(), 1);
        let pages = &s.get(id).unwrap().info.pages;
        assert_eq!(pages.len(), 3);
        assert_eq!((pages[0].width, pages[0].height), (200.0, 300.0), "the cover is untouched");
        let w = |p: &PageInfo| p.width;
        assert!((w(&pages[1]) - 270.0).abs() < 10.0 && (w(&pages[2]) - 330.0).abs() < 10.0, "{} + {}", w(&pages[1]), w(&pages[2]));
        assert!((pages[1].crop[0] - 0.0).abs() < 0.5 && (pages[2].crop[2] - 600.0).abs() < 0.5, "left half first, then the right");
        assert!((pages[1].crop[2] - pages[2].crop[0]).abs() < 0.01, "the halves meet at the binding");
        // One undo puts the spread back.
        assert!(s.undo(id).is_ok());
        assert_eq!(s.get(id).unwrap().info.pages.len(), 2);
    }

    #[test]
    fn a_turned_spread_is_split_as_it_is_shown() {
        // A spread stored portrait and shown landscape (/Rotate 90): the halves are the displayed
        // left and right.
        let mut s = Session::new().with_clock(|| 1_700_000_000);
        let blank = s.create_blank(400.0, 600.0, 1).unwrap();
        let id = s.open_new("turned.pdf", blank).unwrap();
        s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
        let info = s.get(id).unwrap().info.pages[0].clone();
        assert!(is_spread(&info), "{} × {}", info.width, info.height);
        assert_eq!(s.apply_spreads(id, &[Spread { page: 0, at: 0.5 }]).unwrap(), 1);
        let pages = &s.get(id).unwrap().info.pages;
        assert_eq!(pages.len(), 2);
        for p in pages {
            assert!((p.width - 300.0).abs() < 0.5 && (p.height - 400.0).abs() < 0.5, "{} × {}", p.width, p.height);
        }
    }

    #[test]
    fn without_a_binding_to_see_the_page_is_split_in_the_middle() {
        let (w, h) = (480, 320);
        assert_eq!(find_binding(&vec![245u8; w * h], w, h), 0.5, "a blank sheet");
        let g = spread(w, h, [(0.0, 1.0), (0.0, 0.0)], None);
        assert_eq!(find_binding(&g, w, h), 0.5, "text all across");
        // One page of text and a blank one (a verso left empty): no binding between text.
        let g = spread(w, h, [(0.06, 0.44), (0.0, 0.0)], None);
        assert_eq!(find_binding(&g, w, h), 0.5, "a blank page beside a page of text");
        assert_eq!(find_binding(&[], 0, 0), 0.5);
        assert_eq!(find_binding(&[1, 2, 3], 100, 100), 0.5, "short buffer");
    }
}
