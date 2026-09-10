//! Edge-pair sources. Every untyped tier is a stream of `(from, to)` id
//! pairs, whatever file it comes from: a SNAP edge list (`from to` per
//! line, `#` comments), a SNAP temporal edge list (`from to timestamp`,
//! every line an interaction, so a pair may repeat), or a Matrix Market
//! coordinate file inside a `.tar.gz` (SuiteSparse's GAP graphs: `%`
//! comments, a dimensions line, then `i j [value]`, one-based; a
//! `symmetric` matrix lists each undirected edge once and is expanded to
//! both directions here, the way SNAP's road networks list theirs). The
//! materialized and the compact loaders both read from this, so the
//! `LoadStats` a row carries do not depend on which path built them.

use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use flate2::read::GzDecoder;

/// The shape of an untyped dataset file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairFormat {
    /// `from to` per line; exact duplicate pairs are dropped by the loaders.
    SnapEdgeList,
    /// `from to timestamp` per line; a repeated pair is a parallel edge and
    /// is kept: the multigraph is the tier's pathology.
    SnapTemporal,
    /// A Matrix Market coordinate file in a `.tar.gz`; `symmetric` expands
    /// to both directions.
    MatrixMarket,
}

impl PairFormat {
    /// The loader's format name, as recorded in the report.
    pub fn name(self) -> &'static str {
        match self {
            Self::SnapEdgeList => super::SNAP_FORMAT,
            Self::SnapTemporal => super::SNAP_TEMPORAL_FORMAT,
            Self::MatrixMarket => super::MATRIX_MARKET_FORMAT,
        }
    }

    /// Whether a repeated pair is an edge in its own right.
    pub fn keeps_parallel_edges(self) -> bool {
        matches!(self, Self::SnapTemporal)
    }
}

/// A stream of `(from, to)` pairs over one file.
pub struct PairSource {
    reader: Box<dyn BufRead>,
    format: PairFormat,
    /// Matrix Market only: the header has been read and `symmetric` decided.
    header_done: bool,
    symmetric: bool,
    /// The mirror of the last symmetric entry, yielded before the next line.
    pending: Option<(String, String)>,
    /// Physical lines read, comments and headers included.
    pub lines: usize,
    line: String,
}

impl PairSource {
    pub fn open(path: &Path, format: PairFormat) -> io::Result<Self> {
        let reader: Box<dyn BufRead> = match format {
            PairFormat::SnapEdgeList | PairFormat::SnapTemporal => Box::new(
                BufReader::with_capacity(1 << 20, super::open_maybe_gz(path)?),
            ),
            PairFormat::MatrixMarket => Box::new(BufReader::with_capacity(
                1 << 20,
                matrix_market_entry(path)?,
            )),
        };
        Ok(Self {
            reader,
            format,
            header_done: false,
            symmetric: false,
            pending: None,
            lines: 0,
            line: String::new(),
        })
    }

    /// The next pair, or `None` at the end of the file.
    pub fn next_pair(&mut self) -> io::Result<Option<(String, String)>> {
        if let Some(mirror) = self.pending.take() {
            return Ok(Some(mirror));
        }
        loop {
            self.line.clear();
            if self.reader.read_line(&mut self.line)? == 0 {
                return Ok(None);
            }
            self.lines += 1;
            let line = self.line.trim();
            if line.is_empty() {
                continue;
            }
            match self.format {
                PairFormat::SnapEdgeList | PairFormat::SnapTemporal => {
                    if line.starts_with('#') {
                        continue;
                    }
                    let mut parts = line.split(['\t', ' ', ',']).filter(|p| !p.is_empty());
                    let (Some(from), Some(to)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    return Ok(Some((from.to_string(), to.to_string())));
                }
                PairFormat::MatrixMarket => {
                    if let Some(header) = line.strip_prefix("%%MatrixMarket") {
                        let words: Vec<&str> = header.split_whitespace().collect();
                        if words.first().copied() != Some("matrix")
                            || words.get(1).copied() != Some("coordinate")
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("not a coordinate matrix: {line}"),
                            ));
                        }
                        self.symmetric = words
                            .get(3)
                            .is_some_and(|s| s.eq_ignore_ascii_case("symmetric"));
                        continue;
                    }
                    if line.starts_with('%') {
                        continue;
                    }
                    if !self.header_done {
                        // The dimensions line: rows, columns, entries.
                        self.header_done = true;
                        continue;
                    }
                    let mut parts = line.split_whitespace();
                    let (Some(i), Some(j)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    if self.symmetric && i != j {
                        self.pending = Some((j.to_string(), i.to_string()));
                    }
                    return Ok(Some((i.to_string(), j.to_string())));
                }
            }
        }
    }
}

/// The matrix inside a SuiteSparse tarball, streamed: the entry named
/// `<name>/<name>.mtx` (the auxiliary `_sources`/`_coord` files are not the
/// graph), read by a thread into an anonymous pipe so the archive's borrow
/// never reaches the caller.
fn matrix_market_entry(path: &Path) -> io::Result<Box<dyn Read + Send>> {
    let file = std::fs::File::open(path)?;
    let (reader, mut writer) = io::pipe()?;
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("matrix-market-untar".into())
        .spawn(move || {
            let mut archive = tar::Archive::new(GzDecoder::new(file));
            let result = (|| -> io::Result<()> {
                for entry in archive.entries()? {
                    let mut entry = entry?;
                    let entry_path = entry.path()?.into_owned();
                    let is_graph = entry_path.extension().is_some_and(|e| e == "mtx")
                        && entry_path.file_stem().is_some_and(|stem| {
                            entry_path
                                .parent()
                                .and_then(|p| p.file_name())
                                .is_none_or(|dir| dir == stem)
                        });
                    if is_graph {
                        io::copy(&mut entry, &mut writer)?;
                        return Ok(());
                    }
                }
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no <name>/<name>.mtx entry in {}", path.display()),
                ))
            })();
            // A reader that stopped early (a truncated smoke load) closes
            // the pipe; that is not an error of the archive.
            if let Err(e) = result
                && e.kind() != io::ErrorKind::BrokenPipe
            {
                eprintln!("matrix market: {e}");
            }
            // Dropping the writer ends the reader's stream.
        })?;
    Ok(Box::new(reader))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(path: &Path, format: PairFormat) -> (Vec<(String, String)>, usize) {
        let mut src = PairSource::open(path, format).unwrap();
        let mut out = Vec::new();
        while let Some(p) = src.next_pair().unwrap() {
            out.push(p);
        }
        (out, src.lines)
    }

    #[test]
    fn a_temporal_list_keeps_every_interaction_and_drops_the_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sx-t.txt");
        std::fs::write(&p, "# c\n1 2 100\n1 2 200\n2 1 300\n").unwrap();
        let (got, lines) = pairs(&p, PairFormat::SnapTemporal);
        assert_eq!(
            got,
            vec![
                ("1".into(), "2".into()),
                ("1".into(), "2".into()),
                ("2".into(), "1".into())
            ]
        );
        assert_eq!(lines, 4);
        assert!(PairFormat::SnapTemporal.keeps_parallel_edges());
        assert!(!PairFormat::SnapEdgeList.keeps_parallel_edges());
    }

    #[test]
    fn a_symmetric_matrix_market_tarball_expands_to_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let mtx = "%%MatrixMarket matrix coordinate integer symmetric\n% a comment\n3 3 3\n2 1 7\n3 1 7\n3 3 1\n";
        let aux = "%%MatrixMarket matrix coordinate integer general\n1 1 1\n1 1 5\n";
        let tarball = dir.path().join("G.tar.gz");
        {
            let f = std::fs::File::create(&tarball).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut ar = tar::Builder::new(gz);
            for (name, body) in [("G/G_sources.mtx", aux), ("G/G.mtx", mtx)] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                ar.append_data(&mut header, name, body.as_bytes()).unwrap();
            }
            ar.into_inner().unwrap().finish().unwrap();
        }
        let (got, _) = pairs(&tarball, PairFormat::MatrixMarket);
        assert_eq!(
            got,
            vec![
                ("2".into(), "1".into()),
                ("1".into(), "2".into()),
                ("3".into(), "1".into()),
                ("1".into(), "3".into()),
                ("3".into(), "3".into()),
            ],
            "the graph entry, not the auxiliary one; the self-loop once"
        );
    }

    #[test]
    fn a_general_matrix_is_not_mirrored() {
        let dir = tempfile::tempdir().unwrap();
        let mtx = "%%MatrixMarket matrix coordinate pattern general\n2 2 2\n1 2\n2 1\n";
        let tarball = dir.path().join("H.tar.gz");
        {
            let f = std::fs::File::create(&tarball).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut ar = tar::Builder::new(gz);
            let mut header = tar::Header::new_gnu();
            header.set_size(mtx.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            ar.append_data(&mut header, "H/H.mtx", mtx.as_bytes())
                .unwrap();
            ar.into_inner().unwrap().finish().unwrap();
        }
        let (got, _) = pairs(&tarball, PairFormat::MatrixMarket);
        assert_eq!(
            got,
            vec![("1".into(), "2".into()), ("2".into(), "1".into())]
        );
    }
}
