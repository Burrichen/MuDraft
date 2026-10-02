//! Decoding and record parsing. Pure: bytes in, records and row problems out.

use serde::{Deserialize, Serialize};

use super::{MAX_FILE_BYTES, MAX_ROWS};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRecord {
    /// Spreadsheet row number: the header is row 1, the first record row 2.
    pub row_number: u32,
    pub cells: Vec<String>,
    /// Structural problem (e.g. too many values). Such rows cannot be imported.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    pub headers: Vec<String>,
    pub records: Vec<ParsedRecord>,
}

/// UTF-8 (BOM optional) or UTF-16 LE/BE with BOM. Anything else is rejected with a clear
/// message rather than guessed.
pub fn decode(bytes: &[u8]) -> AppResult<String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::validation(
            "CSV file",
            format!(
                "larger than {} MB; split it into smaller files",
                MAX_FILE_BYTES / (1024 * 1024)
            ),
        ));
    }
    let utf16 = |be: bool| -> AppResult<String> {
        let body = &bytes[2..];
        if !body.len().is_multiple_of(2) {
            return Err(AppError::validation(
                "CSV file",
                "UTF-16 data has an odd number of bytes",
            ));
        }
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16(&units)
            .map_err(|_| AppError::validation("CSV file", "invalid UTF-16 text"))
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => utf8(rest),
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        _ => utf8(bytes),
    }
}

fn utf8(bytes: &[u8]) -> AppResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|e| {
        AppError::validation(
            "CSV file",
            format!(
                "not valid UTF-8 (first bad byte at offset {}). Re-save it as “CSV UTF-8”",
                e.utf8_error().valid_up_to()
            ),
        )
    })
}

pub fn parse(bytes: &[u8]) -> AppResult<ParsedFile> {
    let text = decode(bytes)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rows = reader.records();

    let headers: Vec<String> = match rows.next() {
        Some(Ok(h)) => h.iter().map(|c| c.trim().to_owned()).collect(),
        Some(Err(e)) => return Err(AppError::validation("CSV header", e.to_string())),
        None => return Err(AppError::validation("CSV file", "the file is empty")),
    };
    if headers.iter().all(String::is_empty) {
        return Err(AppError::validation(
            "CSV header",
            "the first row must name the columns",
        ));
    }

    let mut records = Vec::new();
    for (index, result) in rows.enumerate() {
        let row_number = u32::try_from(index + 2).unwrap_or(u32::MAX);
        let (cells, problem) = match result {
            Ok(r) => {
                let mut cells: Vec<String> = r.iter().map(|c| c.trim().to_owned()).collect();
                if cells.iter().all(String::is_empty) {
                    continue; // blank line
                }
                let problem = (cells.len() > headers.len()).then(|| {
                    format!(
                        "has {} values but the header has {} columns — check for an unquoted comma",
                        cells.len(),
                        headers.len()
                    )
                });
                // Fewer values than headers means trailing optional columns are empty.
                cells.resize(cells.len().max(headers.len()), String::new());
                (cells, problem)
            }
            Err(e) => (Vec::new(), Some(format!("could not be read: {e}"))),
        };
        if records.len() == MAX_ROWS {
            return Err(AppError::validation(
                "CSV file",
                format!("more than {MAX_ROWS} rows; split it into smaller files"),
            ));
        }
        records.push(ParsedRecord {
            row_number,
            cells,
            problem,
        });
    }
    Ok(ParsedFile { headers, records })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_bom_quotes_commas_newlines_and_unicode() {
        let csv = "\u{feff}Album,Artist,Tags\r\n\
                   \"Homogenic\",Björk,\"art pop; 1990s, favourites\"\r\n\
                   \"Songs, Vol. 1\",\"Simon & Garfunkel\",\"multi\nline\"\r\n\
                   \"Quote \"\"Test\"\"\",Sigur Rós,\r\n\
                   \r\n\
                   東京,Ｙ・Ｙ,\n";
        let p = parse(csv.as_bytes()).unwrap();
        assert_eq!(p.headers, vec!["Album", "Artist", "Tags"]);
        assert_eq!(p.records.len(), 4, "blank lines are skipped");
        assert_eq!(
            p.records[0].cells,
            vec!["Homogenic", "Björk", "art pop; 1990s, favourites"]
        );
        assert_eq!(p.records[1].cells[0], "Songs, Vol. 1");
        assert_eq!(p.records[1].cells[1], "Simon & Garfunkel");
        assert_eq!(p.records[1].cells[2], "multi\nline");
        assert_eq!(p.records[2].cells[0], "Quote \"Test\"");
        assert_eq!(p.records[3].cells[0], "東京");
        assert_eq!(
            p.records[3].row_number, 5,
            "row numbers count records, not lines"
        );
    }

    #[test]
    fn pads_missing_optional_columns_and_flags_extra_values() {
        let p = parse(b"Album,Artist,Year\nA,B\nC,D,1999,extra\n").unwrap();
        assert_eq!(p.records[0].cells, vec!["A", "B", ""]);
        assert!(p.records[0].problem.is_none());
        assert!(
            p.records[1]
                .problem
                .as_deref()
                .unwrap()
                .contains("unquoted comma")
        );
    }

    #[test]
    fn decodes_utf16_and_rejects_other_encodings() {
        let text = "Album,Artist\nMotörhead,Motörhead\n";
        let mut le = vec![0xFF, 0xFE];
        le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(parse(&le).unwrap().records[0].cells[0], "Motörhead");
        let mut be = vec![0xFE, 0xFF];
        be.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(parse(&be).unwrap().records[0].cells[1], "Motörhead");

        let latin1 = b"Album,Artist\nMot\xf6rhead,X\n";
        assert!(parse(latin1).unwrap_err().to_string().contains("UTF-8"));
        assert_eq!(parse(b"").unwrap_err().code(), "validation");
    }

    #[test]
    fn large_files_parse_and_limits_are_enforced() {
        let mut big = String::from("Album,Artist\n");
        for i in 0..MAX_ROWS {
            big.push_str(&format!("Album {i},Artist {}\n", i % 50));
        }
        assert_eq!(parse(big.as_bytes()).unwrap().records.len(), MAX_ROWS);
        big.push_str("One,Too many\n");
        assert!(parse(big.as_bytes()).is_err());
        assert!(decode(&vec![b'a'; MAX_FILE_BYTES + 1]).is_err());
    }
}
