//! Table of contents to links: read the entries of a printed table of contents ("Chapter
//! one ........ 17") from its page's text (a scan's recognised text works too), find the page each
//! one goes to, and turn them into links over the entries and nested bookmarks.
//!
//! A printed page number is not a page of the file: a book's page 17 is often the file's page 25.
//! Each entry's title is looked for as a heading (a line that begins with it) after the table of
//! contents; the offset most of the found entries agree on then places the rest, and outvotes a
//! match that disagrees with it (a running head, a mention in the text). Page labels
//! (`/PageLabels`) win when the file has them. Roman numbers (front matter) are placed only by
//! their headings or labels.

use crate::Document;

/// One entry of a table of contents.
#[derive(Clone, Debug, PartialEq)]
pub struct TocEntry {
    /// The page (0-based) of the table of contents it is printed on.
    pub page: usize,
    /// Its box on that page, user space: the link goes over it.
    pub rect: [f64; 4],
    pub title: String,
    /// The page number as printed ("17", "iv").
    pub printed: String,
    /// 0 for the outermost entries, 1 for those indented under them, and so on.
    pub level: usize,
    /// The page (0-based) of the file it goes to, when found.
    pub target: Option<usize>,
}

/// What [`Document::table_of_contents`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TocScan {
    pub entries: Vec<TocEntry>,
    /// Printed page `n` is page `n + offset` of the file (0-based), as found or as given.
    pub offset: Option<i64>,
}

/// The most entries read from one table of contents.
const MAX_ENTRIES: usize = 2_000;
/// How many words of a title identify its heading.
const KEY_WORDS: usize = 5;

/// A line of text across the page: the text lines that share a baseline, left to right.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub text: String,
    pub rect: [f64; 4],
}

/// Join a page's text lines into rows: lines whose vertical middles are within half a line of
/// each other, top to bottom.
pub fn rows(lines: &[([f64; 4], String)]) -> Vec<Row> {
    /// A text line: its box and text.
    type Line<'a> = &'a ([f64; 4], String);
    let mut sorted: Vec<Line> = lines.iter().filter(|(r, t)| r.iter().all(|v| v.is_finite()) && r[3] > r[1] && !t.trim().is_empty()).collect();
    sorted.sort_by(|a, b| mid(&b.0).total_cmp(&mid(&a.0)));
    let mut rows: Vec<(f64, f64, Vec<Line>)> = Vec::new();
    for line in sorted {
        let (m, h) = (mid(&line.0), line.0[3] - line.0[1]);
        match rows.last_mut() {
            Some((rm, rh, members)) if (m - *rm).abs() < 0.5 * rh.min(h) => members.push(line),
            _ => rows.push((m, h, vec![line])),
        }
    }
    rows.into_iter()
        .map(|(_, _, mut members)| {
            members.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
            let text = members.iter().map(|(_, t)| t.trim()).collect::<Vec<_>>().join(" ");
            let rect = members
                .iter()
                .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |r, (b, _)| [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])]);
            Row { text, rect }
        })
        .collect()
}

fn mid(r: &[f64; 4]) -> f64 {
    (r[1] + r[3]) / 2.0
}

/// Leaders and separators between a title and its page number.
fn is_leader(c: char) -> bool {
    c.is_whitespace() || matches!(c, '.' | '·' | '…' | '_' | '-' | '–' | '—' | '•' | '‧' | ',')
}

/// A printed page number: 1–4 digits, or a Roman number.
fn page_number(s: &str) -> Option<String> {
    let s = s.trim();
    if (1..=4).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_owned());
    }
    roman(s).map(|_| s.to_owned())
}

/// The value of a Roman number (i–mmmm), either case.
fn roman(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 12 {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let mut total = 0u32;
    let mut prev = 0u32;
    for c in lower.chars().rev() {
        let v = match c {
            'i' => 1,
            'v' => 5,
            'x' => 10,
            'l' => 50,
            'c' => 100,
            'd' => 500,
            'm' => 1000,
            _ => return None,
        };
        if v < prev {
            total = total.checked_sub(v)?;
        } else {
            total += v;
            prev = v;
        }
    }
    // Only canonical spellings: "iiii" or "vx" are words, not numbers.
    (to_roman(total) == lower).then_some(total)
}

fn to_roman(mut n: u32) -> String {
    const PARTS: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (v, s) in PARTS {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// A row read as an entry: (title, printed page), or `None` when it ends in no page number.
fn split_entry(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim_end();
    let last = trimmed.rsplit(char::is_whitespace).next()?;
    // "Introdução....3": the number may be glued to the leaders.
    let number_start = last.char_indices().rev().take_while(|(_, c)| c.is_ascii_alphanumeric()).last().map(|(i, _)| i)?;
    let number = last.get(number_start..)?;
    let printed = page_number(number)?;
    let head = trimmed.get(..trimmed.len() - number.len())?;
    // A Roman number needs leaders (dots, a dash) before it, or "Capítulo I" would read as page 1.
    let leaders = head.get(head.trim_end_matches(is_leader).len()..).unwrap_or_default();
    if !number.chars().all(|c| c.is_ascii_digit()) && !leaders.chars().any(|c| !c.is_whitespace()) {
        return None;
    }
    let title = head.trim_end_matches(is_leader).trim().to_owned();
    if title.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return None;
    }
    Some((title, printed))
}

/// Text folded for matching: lower case, letters and digits only (accents dropped), single spaces.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'ç' | 'Ç' => 'c',
            'ñ' | 'Ñ' => 'n',
            c => c,
        };
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim_end().to_owned()
}

/// The entries of a table of contents printed on `page`, whose rows are `rows` (top to bottom):
/// levels still relative to this page alone, targets not yet found.
pub fn entries_of(page: usize, rows: &[Row]) -> Vec<TocEntry> {
    let mut out: Vec<TocEntry> = Vec::new();
    // A title that wraps: the rows before the one with the number, when they line up with it.
    let mut pending: Vec<&Row> = Vec::new();
    for row in rows {
        match split_entry(&row.text) {
            Some((title, printed)) => {
                let height = row.rect[3] - row.rect[1];
                let joined: Vec<&Row> =
                    pending.iter().copied().filter(|p| p.rect[1] - row.rect[3] < 1.5 * height && p.rect[0] <= row.rect[0] + height).collect();
                let mut text = joined.iter().map(|p| p.text.trim()).collect::<Vec<_>>();
                text.push(&title);
                let rect = joined.iter().fold(row.rect, |r, p| [r[0].min(p.rect[0]), r[1].min(p.rect[1]), r[2].max(p.rect[2]), r[3].max(p.rect[3])]);
                // A word broken at the end of a line ("jurí-" "dica") is joined again.
                let title = text.join(" ").replace("- ", "");
                let left = joined.first().map_or(row.rect[0], |p| p.rect[0]);
                out.push(TocEntry { page, rect, title, printed, level: 0, target: None });
                if let Some(e) = out.last_mut() {
                    e.rect[0] = left.min(e.rect[0]);
                }
                pending.clear();
                if out.len() >= MAX_ENTRIES {
                    break;
                }
            }
            None => {
                pending.push(row);
                if pending.len() > 3 {
                    pending.remove(0);
                }
            }
        }
    }
    out
}

/// Nest entries by indentation: equal left edges (within half a line) share a level, and each
/// step to the right is one level deeper.
pub fn nest(entries: &mut [TocEntry]) {
    let mut lefts: Vec<(f64, f64)> = entries.iter().map(|e| (e.rect[0], ((e.rect[3] - e.rect[1]) * 0.5).max(2.0))).collect();
    lefts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut steps: Vec<f64> = Vec::new();
    for (x, tolerance) in lefts {
        if steps.last().is_none_or(|s| x - s > tolerance) {
            steps.push(x);
        }
    }
    for e in entries.iter_mut() {
        e.level = steps.iter().rposition(|s| *s <= e.rect[0] + 0.5).unwrap_or(0).min(4);
    }
}

/// Find the pages entries go to. `pages` holds each page's rows (folded text), `after` the first
/// page a heading may be on, `labels` the page labels when the file has its own, `offset` an
/// offset to use instead of finding one.
pub fn place(entries: &mut [TocEntry], pages: &[Vec<String>], after: usize, labels: Option<&[String]>, offset: Option<i64>) -> Option<i64> {
    let n = pages.len();
    for e in entries.iter_mut() {
        e.target = None;
    }
    // Labels first: they are the book's own numbering.
    if let Some(labels) = labels {
        for e in entries.iter_mut() {
            e.target = labels.iter().position(|l| l.eq_ignore_ascii_case(&e.printed));
        }
        if entries.iter().all(|e| e.target.is_some()) {
            return offset;
        }
    }
    // Headings: the first page after the contents with a row that begins with the title.
    let heading = |title: &str| {
        let key_full = fold(title);
        let words: Vec<&str> = key_full.split(' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return None;
        }
        let key = words.iter().take(KEY_WORDS).copied().collect::<Vec<_>>().join(" ");
        let short = words.len() < 3;
        (after..n).find(|&p| {
            pages.get(p).is_some_and(|rows| {
                rows.iter().any(|r| {
                    if short {
                        r == &key_full || r.ends_with(&format!(" {key_full}")) && r.split(' ').count() <= words.len() + 2
                    } else {
                        r.starts_with(&key) || r.contains(&format!(" {key}")) && r.split(' ').count() <= words.len() + 4
                    }
                })
            })
        })
    };
    let found: Vec<Option<usize>> = entries.iter().map(|e| if e.target.is_some() { e.target } else { heading(&e.title) }).collect();
    let arabic = |e: &TocEntry| e.printed.parse::<i64>().ok();
    let offset = offset.or_else(|| {
        let mut votes: Vec<(i64, usize)> = Vec::new();
        for (e, f) in entries.iter().zip(&found) {
            if let (Some(p), Some(f)) = (arabic(e), f) {
                let o = *f as i64 - p;
                match votes.iter_mut().find(|(v, _)| *v == o) {
                    Some((_, c)) => *c += 1,
                    None => votes.push((o, 1)),
                }
            }
        }
        votes.into_iter().max_by_key(|(o, c)| (*c, -o.abs())).map(|(o, _)| o)
    });
    for (e, f) in entries.iter_mut().zip(found) {
        let by_offset = arabic(e).zip(offset).and_then(|(p, o)| usize::try_from(p.checked_add(o)?).ok()).filter(|t| *t < n);
        e.target = match (by_offset, f) {
            // The book's numbering outvotes a heading found elsewhere (a running head, a mention).
            (Some(t), Some(f)) if t.abs_diff(f) > 1 => Some(t),
            (_, Some(f)) => Some(f),
            (t, None) => t,
        };
    }
    offset
}

impl Document {
    /// The entries of the table of contents printed on `toc_pages` (0-based), with the page each
    /// goes to. `offset`: printed page `n` is file page `n + offset`; `None` finds it.
    pub fn table_of_contents(&self, toc_pages: &[usize], offset: Option<i64>) -> TocScan {
        let n = self.info.pages.len();
        let mut entries = Vec::new();
        for &p in toc_pages.iter().filter(|p| **p < n) {
            let lines: Vec<([f64; 4], String)> = self.text_lines(p).into_iter().map(|l| (l.rect, l.text)).collect();
            entries.extend(entries_of(p, &rows(&lines)));
            if entries.len() >= MAX_ENTRIES {
                entries.truncate(MAX_ENTRIES);
                break;
            }
        }
        nest(&mut entries);
        let after = toc_pages.iter().copied().filter(|p| *p < n).max().map_or(0, |p| p + 1);
        let pages: Vec<Vec<String>> = (0..n)
            .map(|p| {
                if p < after {
                    return Vec::new();
                }
                let lines: Vec<([f64; 4], String)> = self.text_lines(p).into_iter().map(|l| (l.rect, l.text)).collect();
                rows(&lines).into_iter().map(|r| fold(&r.text)).filter(|t| !t.is_empty()).collect()
            })
            .collect();
        let own_labels = self.info.pages.iter().enumerate().any(|(i, p)| p.label != (i + 1).to_string());
        let labels: Option<Vec<String>> = own_labels.then(|| self.info.pages.iter().map(|p| p.label.clone()).collect());
        let offset = place(&mut entries, &pages, after, labels.as_deref(), offset);
        TocScan { entries, offset }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str, x: f64, y: f64) -> Row {
        Row { text: text.into(), rect: [x, y, 400.0, y + 10.0] }
    }

    #[test]
    fn entries_keep_their_titles_numbers_and_indentation() {
        let rows = [
            row("ÍNDICE", 200.0, 700.0),
            row("Págs.", 380.0, 680.0),
            row("Prefácio . . . . . . . . . . . . . . . I", 100.0, 660.0),
            row("Introdução . . . . . . . . . . . . . 3", 100.0, 640.0),
            row("II — O discurso jurídico em Pasárgada . . . . 17", 100.0, 620.0),
            row("II.1 — A retórica da decisão . . . . . 17", 120.0, 605.0),
            row("IV — A correlação sociológica entre as dimensões da instância jurídica e seus", 100.0, 585.0),
            row("problemas . . . . . . . . . . . . 62", 110.0, 572.0),
            row("Capítulo I", 100.0, 550.0),
        ];
        let mut e = entries_of(4, &rows);
        nest(&mut e);
        let got: Vec<(&str, &str, usize)> = e.iter().map(|e| (e.title.as_str(), e.printed.as_str(), e.level)).collect();
        assert_eq!(
            got,
            [
                ("Prefácio", "I", 0),
                ("Introdução", "3", 0),
                ("II — O discurso jurídico em Pasárgada", "17", 0),
                ("II.1 — A retórica da decisão", "17", 1),
                ("IV — A correlação sociológica entre as dimensões da instância jurídica e seus problemas", "62", 0),
            ]
        );
        assert_eq!(e[4].rect[3], 595.0, "the wrapped entry's box covers both rows");
        assert_eq!(split_entry("Introdução....3"), Some(("Introdução".into(), "3".into())));
        assert_eq!(split_entry("Capítulo I"), None, "a Roman number glued to a word is part of the title");
        assert_eq!(split_entry("Em 1988"), Some(("Em".into(), "1988".into())), "a bare number still ends an entry");
    }

    #[test]
    fn headings_and_the_shared_offset_place_the_entries() {
        let mut e: Vec<TocEntry> = [("Prefácio", "i"), ("Introdução", "3"), ("O discurso jurídico", "17"), ("Conclusão", "40")]
            .into_iter()
            .map(|(t, p)| TocEntry { page: 2, rect: [0.0; 4], title: t.into(), printed: p.into(), level: 0, target: None })
            .collect();
        // 50 pages; the book's page n is file page n + 6 (0-based). "Conclusão" has no heading
        // of its own; "O discurso jurídico" is also a running head from page 24 on, after the
        // chapter's first page 23.
        let mut pages: Vec<Vec<String>> = vec![Vec::new(); 50];
        pages[4] = vec!["prefacio".into()];
        pages[9] = vec!["introducao".into(), "o texto".into()];
        pages[23] = vec!["ii o discurso juridico".into()];
        pages[24] = vec!["o discurso juridico 18".into()];
        let offset = place(&mut e, &pages, 3, None, None);
        assert_eq!(offset, Some(6));
        let targets: Vec<Option<usize>> = e.iter().map(|e| e.target).collect();
        assert_eq!(targets, [Some(4), Some(9), Some(23), Some(46)]);
        // An offset given by the user wins.
        let offset = place(&mut e, &pages, 3, None, Some(7));
        assert_eq!(offset, Some(7));
        assert_eq!(e[3].target, Some(47));
    }

    #[test]
    fn roman_numbers_are_only_canonical() {
        assert_eq!(roman("iv"), Some(4));
        assert_eq!(roman("XIV"), Some(14));
        assert_eq!(roman("iiii"), None);
        assert_eq!(roman("vx"), None);
        assert_eq!(roman("mix"), Some(1009));
        assert_eq!(fold("II.1 — A Retórica  da Decisão"), "ii 1 a retorica da decisao");
    }
}
