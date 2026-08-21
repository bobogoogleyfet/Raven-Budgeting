//! Just enough of the xlsx format to read one published worksheet.
//!
//! # Why not a spreadsheet library
//!
//! This reads exactly one file: the Utah Tax Commission's quarterly rate
//! workbook. It needs cell text from named sheets and nothing else — no
//! formulas, no styles, no dates, no charts, no writing. A general reader
//! brings all of that plus a dependency tree, and on the two platforms this app
//! is aimed at next (Android, iOS) every added tree is another thing that has
//! to cross-compile.
//!
//! An xlsx is a zip of XML, and both of those are already in the tree. What is
//! here is the subset that a rate table exercises, and it fails loudly rather
//! than guessing when it meets anything else.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{CoreError, Result};

/// A worksheet as rows of `column letter -> cell text`.
///
/// One `String` per cell whatever its type: a shared string arrives resolved,
/// a number arrives as the literal the file stores (`7.25E-2`). Interpreting
/// that is the caller's job, because only the caller knows which column is a
/// rate and which is a name.
pub(crate) type Sheet = Vec<BTreeMap<String, String>>;

fn bad(message: impl Into<String>) -> CoreError {
    CoreError::RateData(message.into())
}

/// A text event's actual characters.
///
/// Decoding and entity expansion are two steps: the reader hands back bytes in
/// the file's encoding, and `&amp;` is still four characters at that point.
/// Skipping the second step would file a rate under "Alta &amp; District".
fn text_of(t: &quick_xml::events::BytesText<'_>) -> Result<String> {
    let decoded = t
        .xml10_content()
        .map_err(|e| bad(format!("undecodable text: {e}")))?;
    Ok(quick_xml::escape::unescape(&decoded)
        .map_err(|e| bad(format!("bad escape: {e}")))?
        .into_owned())
}

/// Read one named worksheet out of an xlsx in memory.
///
/// The sheet is found by the name shown on its tab, resolved through the
/// workbook's relationships the way a spreadsheet program does. Reaching
/// straight for `xl/worksheets/sheet1.xml` would be shorter and would break
/// silently the first time the publisher inserts a sheet — the file is
/// rebuilt by hand every quarter, and sheet *order* is exactly the kind of
/// thing that moves.
pub(crate) fn read_sheet(bytes: &[u8], sheet_name: &str) -> Result<Sheet> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| bad(format!("not a readable xlsx: {e}")))?;

    let workbook = entry(&mut zip, "xl/workbook.xml")?;
    let rel_id = sheet_relationship_id(&workbook, sheet_name)?;
    let rels = entry(&mut zip, "xl/_rels/workbook.xml.rels")?;
    let target = relationship_target(&rels, &rel_id)?;

    let shared = match entry(&mut zip, "xl/sharedStrings.xml") {
        Ok(xml) => shared_strings(&xml)?,
        // A workbook whose every cell is inline or numeric has no such part.
        // Absent is not corrupt.
        Err(_) => Vec::new(),
    };

    let path = format!("xl/{}", target.trim_start_matches('/'));
    let sheet = entry(&mut zip, &path)?;
    cells(&sheet, &shared)
}

/// Read one zip member to a string.
fn entry<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Result<String> {
    let mut file = zip
        .by_name(name)
        .map_err(|_| bad(format!("the workbook has no {name}")))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| bad(format!("{name} is not readable text: {e}")))?;
    Ok(text)
}

/// The `r:id` of the sheet with this tab name.
fn sheet_relationship_id(workbook_xml: &str, sheet_name: &str) -> Result<String> {
    let mut reader = Reader::from_str(workbook_xml);
    let mut buf = Vec::new();
    let mut names = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == b"sheet" => {
                let name = attribute(&e, b"name").unwrap_or_default();
                let id = attribute(&e, b"id").unwrap_or_default();
                if name.eq_ignore_ascii_case(sheet_name) && !id.is_empty() {
                    return Ok(id);
                }
                names.push(name);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(bad(format!("workbook.xml is malformed: {e}"))),
        }
        buf.clear();
    }
    // Naming what *was* there turns "the download changed shape" from a
    // mystery into a one-line diagnosis.
    Err(bad(format!(
        "no sheet named {sheet_name:?}; the workbook has {names:?}"
    )))
}

/// Where a relationship id points, relative to `xl/`.
fn relationship_target(rels_xml: &str, rel_id: &str) -> Result<String> {
    let mut reader = Reader::from_str(rels_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == b"Relationship" => {
                if attribute(&e, b"Id").as_deref() == Some(rel_id)
                    && let Some(target) = attribute(&e, b"Target")
                {
                    return Ok(target);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(bad(format!("workbook.xml.rels is malformed: {e}"))),
        }
        buf.clear();
    }
    Err(bad(format!("relationship {rel_id} points nowhere")))
}

/// The shared string table, in index order.
///
/// A `<si>` can be split across several `<r>` runs when part of the text is
/// styled differently; the string is their concatenation, so every `<t>` under
/// one `<si>` is joined rather than the first one winning.
fn shared_strings(xml: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut strings = Vec::new();
    let mut current: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"si" => {
                current = Some(String::new())
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"si" => {
                strings.push(current.take().unwrap_or_default());
            }
            Ok(Event::Text(t)) => {
                if let Some(slot) = current.as_mut() {
                    slot.push_str(&text_of(&t)?);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(bad(format!("sharedStrings.xml is malformed: {e}"))),
        }
        buf.clear();
    }
    Ok(strings)
}

/// Every non-empty cell, grouped by row and keyed by column letter.
fn cells(xml: &str, shared: &[String]) -> Result<Sheet> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut sheet: Sheet = Vec::new();
    let mut row: BTreeMap<String, String> = BTreeMap::new();
    // The cell currently open: its column letter and whether <v> holds an
    // index into the shared table rather than a literal.
    let mut column = String::new();
    let mut is_shared = false;
    let mut in_value = false;
    let mut value = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == b"c" => {
                column = attribute(&e, b"r").map(column_of).unwrap_or_default();
                is_shared = attribute(&e, b"t").as_deref() == Some("s");
                value.clear();
            }
            // `<v>` is the stored value; `<t>` is an inline string's text.
            Ok(Event::Start(e))
                if matches!(e.local_name().as_ref(), b"v" | b"t") && !column.is_empty() =>
            {
                in_value = true;
            }
            Ok(Event::Text(t)) if in_value => {
                value.push_str(&text_of(&t)?);
            }
            Ok(Event::End(e)) if matches!(e.local_name().as_ref(), b"v" | b"t") => in_value = false,
            Ok(Event::End(e)) if e.local_name().as_ref() == b"c" => {
                if !column.is_empty() && !value.is_empty() {
                    let resolved = if is_shared {
                        let index: usize = value
                            .trim()
                            .parse()
                            .map_err(|_| bad(format!("{value:?} is not a shared-string index")))?;
                        shared
                            .get(index)
                            .ok_or_else(|| bad(format!("shared string {index} is out of range")))?
                            .clone()
                    } else {
                        value.clone()
                    };
                    row.insert(std::mem::take(&mut column), resolved);
                }
                column.clear();
                value.clear();
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"row" => {
                // Blank rows are dropped rather than carried: the source uses
                // them as spacing between counties, and a caller counting rows
                // should be counting jurisdictions.
                if !row.is_empty() {
                    sheet.push(std::mem::take(&mut row));
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(bad(format!("the worksheet is malformed: {e}"))),
        }
        buf.clear();
    }
    Ok(sheet)
}

/// `"AB12"` → `"AB"`.
fn column_of(reference: String) -> String {
    reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect()
}

/// An attribute's value, matched on local name so a namespace prefix (`r:id`)
/// does not have to be spelled out.
fn attribute(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name)
            .then(|| String::from_utf8_lossy(a.value.as_ref()).into_owned())
    })
}
