//! LDBC SNB Interactive v1, CsvBasic layout with the long date formatter:
//! `|`-separated files with a header, one per entity (`dynamic/person_0_0.csv`,
//! `static/place_0_0.csv`) or relationship (`dynamic/person_knows_person_0_0.csv`),
//! dates as millisecond epochs. Node ids are namespaced by label
//! (`Person:933`) because SNB ids repeat across entity types; the raw id
//! stays as the `sourceId` property (Grust mirrors a node's identity as
//! its `id` property, so the source column keeps its own name). Posts and
//! comments are one label,
//! `Message`, with `kind` = `Post` or `Comment`, and `organisation` and
//! `place` split into the labels their `type` column names (Company,
//! University; Continent, Country, City): exactly the LSQB projected-FK
//! shape, so the LSQB queries and their digests carry over. Relationship
//! types are the file's verb in upper snake case (`KNOWS`, `HAS_CREATOR`),
//! and the multi-valued attribute files (`person_speaks_language`,
//! `person_email_emailaddress`) become string-array properties.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use grust::{Graph, Props, Value};

use super::LoadStats;
use super::typed::{
    DatasetSchema, TypedGraphBuilder, integer_columns, relationship_label, typed_value,
};

pub const FORMAT: &str = "ldbc-snb-csvbasic";
const DATE_COLUMNS: &[&str] = &["creationDate", "birthday", "joinDate"];

/// The archive's top-level directory, extracted beside it on first use
/// (`tar --zstd`; the `.crc` sidecars are ignored).
pub fn extracted_dir(archive: &Path) -> std::io::Result<PathBuf> {
    let stem = archive
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".tar.zst"))
        .ok_or_else(|| std::io::Error::other("SNB archive is not a .tar.zst"))?;
    let parent = archive.parent().unwrap_or(Path::new("."));
    let directory = parent.join(stem);
    if directory.join("dynamic").is_dir() && directory.join("static").is_dir() {
        return Ok(directory);
    }
    let status = Command::new("tar")
        .args(["--zstd", "-xf"])
        .arg(archive)
        .arg("-C")
        .arg(parent)
        .status()?;
    if !status.success() || !directory.join("dynamic").is_dir() {
        return Err(std::io::Error::other(format!(
            "extracting {} with `tar --zstd` failed (is zstd installed?)",
            archive.display()
        )));
    }
    Ok(directory)
}

pub fn load(
    archive: &Path,
    limit: Option<usize>,
) -> std::io::Result<(Graph, LoadStats, DatasetSchema)> {
    let directory = extracted_dir(archive)?;
    let mut builder = TypedGraphBuilder::new(limit);
    let files = csv_files(&directory)?;
    let is_node = |stem: &str| !stem.contains('_');
    let is_attribute = |stem: &str| stem.contains("_speaks_") || stem.contains("_email_");
    for (stem, path) in files.iter().filter(|(stem, _)| is_node(stem)) {
        load_nodes(&mut builder, stem, path)?;
    }
    // A slice keeps every relationship type in proportion: each file gets
    // the share of the limit its row count is of the whole, so a 200k slice
    // is not just the first files in name order (which would hold comments
    // and almost no KNOWS edges).
    let edge_files: Vec<&(String, PathBuf)> = files
        .iter()
        .filter(|(stem, _)| !is_node(stem) && !is_attribute(stem))
        .collect();
    let rows = edge_files
        .iter()
        .map(|(_, path)| row_count(path))
        .collect::<std::io::Result<Vec<_>>>()?;
    let caps = proportional_caps(limit, &rows);
    let mut attributes: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for (stem, path) in files.iter().filter(|(stem, _)| is_attribute(stem)) {
        if let [entity, _, attribute] = stem.split('_').collect::<Vec<_>>().as_slice() {
            load_attribute(&mut attributes, entity, attribute, path)?;
        }
    }
    for ((stem, path), cap) in edge_files.iter().zip(caps) {
        if let [source, verb, target] = stem.split('_').collect::<Vec<_>>().as_slice() {
            load_edges(&mut builder, source, verb, target, path, cap)?;
        }
    }
    let (mut graph, mut stats, schema) = builder.finish(archive.display().to_string(), FORMAT);
    attach_attributes(&mut graph, attributes);
    stats.file = archive.display().to_string();
    Ok((graph, stats, schema))
}

/// Every data file as `(stem, path)`: `dynamic/person_0_0.csv` → `person`,
/// sorted so node files load before relationship files and the order is
/// stable across hosts.
fn csv_files(directory: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    for sub in ["static", "dynamic"] {
        for entry in std::fs::read_dir(directory.join(sub))? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || !name.ends_with(".csv") {
                continue;
            }
            let stem = name.trim_end_matches(".csv").trim_end_matches("_0_0");
            files.push((stem.to_string(), path));
        }
    }
    files.sort_by(|a, b| {
        a.0.contains('_')
            .cmp(&b.0.contains('_'))
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(files)
}

fn reader(path: &Path) -> std::io::Result<csv::Reader<std::fs::File>> {
    Ok(csv::ReaderBuilder::new()
        .delimiter(b'|')
        .quoting(false)
        .flexible(true)
        .from_path(path)?)
}

/// `Person`; `Message` for posts and comments; or the label a `type` column
/// names for organisations and places.
fn node_label(entity: &str, record: &csv::StringRecord, headers: &csv::StringRecord) -> String {
    if matches!(entity, "post" | "comment") {
        return "Message".to_string();
    }
    if matches!(entity, "organisation" | "place")
        && let Some(position) = headers.iter().position(|h| h == "type")
        && let Some(kind) = record.get(position)
    {
        return capitalize(kind);
    }
    capitalize(entity)
}

/// The label an entity file or endpoint column names: `person` → `Person`,
/// and `tagclass` → `TagClass`, which is how the endpoint columns
/// (`TagClass.id`) and the LSQB projected-FK layout spell it.
fn capitalize(word: &str) -> String {
    if word.eq_ignore_ascii_case("tagclass") {
        return "TagClass".to_string();
    }
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// The namespace an edge endpoint column refers to: `Person.id` → `Person`,
/// and `Organisation.id` / `Place.id` → the split labels are resolved by
/// looking the raw id up under each candidate.
fn namespaced(builder: &TypedGraphBuilder, column_type: &str, raw_id: &str) -> Option<String> {
    let candidates: &[&str] = match column_type {
        "Organisation" => &["Company", "University"],
        "Place" => &["City", "Country", "Continent"],
        "Post" | "Comment" => return Some(format!("Message:{raw_id}")),
        other => return Some(format!("{other}:{raw_id}")),
    };
    candidates
        .iter()
        .map(|label| format!("{label}:{raw_id}"))
        .find(|id| builder.has_node(id))
}

fn load_nodes(builder: &mut TypedGraphBuilder, entity: &str, path: &Path) -> std::io::Result<()> {
    let mut reader = reader(path)?;
    let headers = reader.headers()?.clone();
    let records: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;
    let int_columns = integer_columns(&headers, &records);
    for record in records {
        builder.lines += 1;
        let Some(raw_id) = record.get(0) else {
            continue;
        };
        let label = node_label(entity, &record, &headers);
        let mut props = Props::new();
        if label == "Message" {
            props.insert("kind".to_string(), Value::String(capitalize(entity)));
        }
        for (i, (column, cell)) in headers.iter().zip(record.iter()).enumerate() {
            if column == "type" && matches!(entity, "organisation" | "place") {
                continue;
            }
            let column = if column == "id" { "sourceId" } else { column };
            if let Some(value) = typed_value(column, cell, int_columns[i], DATE_COLUMNS) {
                props.insert(column.to_string(), value);
            }
        }
        builder.node(&label, format!("{label}:{raw_id}"), props);
    }
    Ok(())
}

/// Data rows in a `|`-separated file (every line but the header).
fn row_count(path: &Path) -> std::io::Result<usize> {
    use std::io::BufRead;
    let lines = std::io::BufReader::new(std::fs::File::open(path)?)
        .lines()
        .count();
    Ok(lines.saturating_sub(1))
}

/// Per-file edge caps that share `limit` in proportion to the files' row
/// counts (rounding up, so small files keep a few rows); `None` is no cap.
fn proportional_caps(limit: Option<usize>, rows: &[usize]) -> Vec<Option<usize>> {
    let total: usize = rows.iter().sum();
    match limit {
        Some(limit) if total > limit => rows
            .iter()
            .map(|&count| Some(((count as u128 * limit as u128).div_ceil(total as u128)) as usize))
            .collect(),
        _ => vec![None; rows.len()],
    }
}

fn load_edges(
    builder: &mut TypedGraphBuilder,
    source: &str,
    verb: &str,
    target: &str,
    path: &Path,
    cap: Option<usize>,
) -> std::io::Result<()> {
    let label = relationship_label(verb);
    let mut reader = reader(path)?;
    let headers = reader.headers()?.clone();
    let source_type = capitalize(source);
    let target_type = capitalize(target);
    let _ = (&source_type, &target_type);
    let column_type = |column: &str| column.split('.').next().unwrap_or(column).to_string();
    let from_type = headers.get(0).map(column_type).unwrap_or_default();
    let to_type = headers.get(1).map(column_type).unwrap_or_default();
    let mut taken = 0usize;
    let records: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;
    let int_columns = integer_columns(&headers, &records);
    for record in records {
        if cap.is_some_and(|cap| taken >= cap) || builder.full() {
            break;
        }
        builder.lines += 1;
        let (Some(from_raw), Some(to_raw)) = (record.get(0), record.get(1)) else {
            continue;
        };
        let (Some(from), Some(to)) = (
            namespaced(builder, &from_type, from_raw),
            namespaced(builder, &to_type, to_raw),
        ) else {
            builder.dangling += 1;
            continue;
        };
        let mut props = Props::new();
        for (i, (column, cell)) in headers.iter().zip(record.iter()).enumerate().skip(2) {
            if let Some(value) = typed_value(column, cell, int_columns[i], DATE_COLUMNS) {
                props.insert(column.to_string(), value);
            }
        }
        if builder.edge(&label, &from, &to, props) {
            taken += 1;
        }
    }
    Ok(())
}

fn load_attribute(
    attributes: &mut BTreeMap<String, BTreeMap<String, Vec<String>>>,
    entity: &str,
    attribute: &str,
    path: &Path,
) -> std::io::Result<()> {
    let mut reader = reader(path)?;
    let key = match attribute {
        "language" => "speaks",
        "emailaddress" => "email",
        other => other,
    }
    .to_string();
    let label = capitalize(entity);
    for record in reader.records() {
        let record = record?;
        let (Some(raw_id), Some(value)) = (record.get(0), record.get(1)) else {
            continue;
        };
        attributes
            .entry(format!("{label}:{raw_id}"))
            .or_default()
            .entry(key.clone())
            .or_default()
            .push(value.to_string());
    }
    Ok(())
}

fn attach_attributes(
    graph: &mut Graph,
    attributes: BTreeMap<String, BTreeMap<String, Vec<String>>>,
) {
    if attributes.is_empty() {
        return;
    }
    let wanted: HashSet<&str> = attributes.keys().map(String::as_str).collect();
    for node in graph
        .nodes
        .iter_mut()
        .filter(|node| wanted.contains(node.id.as_str()))
    {
        if let Some(values) = attributes.get(node.id.as_str()) {
            for (key, list) in values {
                node.props
                    .insert(key.clone(), Value::StringArray(list.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, body: &str| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        write(
            "dynamic/person_0_0.csv",
            "id|firstName|lastName|gender|birthday|creationDate|locationIP|browserUsed\n933|Mahinda|Perera|male|628646400000|1266161530447|119.235.7.103|Firefox\n1|Alan|Turing|male|628646400000|1266161530447|1.1.1.1|Chrome\n",
        );
        write(
            "dynamic/person_knows_person_0_0.csv",
            "Person.id|Person.id|creationDate\n933|1|1271939457947\n1|933|1271939457947\n",
        );
        write(
            "dynamic/person_speaks_language_0_0.csv",
            "Person.id|language\n933|si\n933|en\n",
        );
        write(
            "dynamic/person_workAt_organisation_0_0.csv",
            "Person.id|Organisation.id|workFrom\n933|1226|2013\n933|9999|2013\n",
        );
        write(
            "dynamic/comment_0_0.csv",
            "id|creationDate|locationIP|browserUsed|content|length\n7|1313591219961|46.16.217.105|Chrome|yes|3\n",
        );
        write(
            "dynamic/comment_hasCreator_person_0_0.csv",
            "Comment.id|Person.id\n7|933\n",
        );
        write(
            "static/organisation_0_0.csv",
            "id|type|name|url\n1226|company|Kam_Air|http://dbpedia.org/resource/Kam_Air\n1|university|MIT|http://x\n",
        );
        write(
            "static/place_0_0.csv",
            "id|name|url|type\n933|India|http://dbpedia.org/resource/India|country\n",
        );
        write(
            "static/organisation_isLocatedIn_place_0_0.csv",
            "Organisation.id|Place.id\n1226|933\n",
        );
        write(
            "static/tagclass_0_0.csv",
            "id|name|url\n349|OfficeHolder|http://x\n",
        );
        write(
            "static/tag_0_0.csv",
            "id|name|url\n5|Hamid_Karzai|http://x\n",
        );
        write(
            "static/tag_hasType_tagclass_0_0.csv",
            "Tag.id|TagClass.id\n5|349\n",
        );
        dir
    }

    #[test]
    fn proportional_caps_share_the_limit_by_row_count() {
        assert_eq!(proportional_caps(None, &[10, 20]), vec![None, None]);
        assert_eq!(proportional_caps(Some(100), &[10, 20]), vec![None, None]);
        assert_eq!(
            proportional_caps(Some(30), &[90, 10, 1]),
            vec![Some(27), Some(3), Some(1)]
        );
    }

    #[test]
    fn typed_nodes_edges_and_attributes() {
        let dir = fixture();
        let mut builder = TypedGraphBuilder::new(None);
        let files = csv_files(dir.path()).unwrap();
        assert!(
            files.iter().position(|f| f.0 == "person").unwrap()
                < files
                    .iter()
                    .position(|f| f.0 == "person_knows_person")
                    .unwrap()
        );
        for (stem, path) in &files {
            if !stem.contains('_') {
                load_nodes(&mut builder, stem, path).unwrap();
            }
        }
        let mut attributes = BTreeMap::new();
        for (stem, path) in &files {
            let parts: Vec<&str> = stem.split('_').collect();
            match parts.as_slice() {
                [e, v, a] if *v == "speaks" => load_attribute(&mut attributes, e, a, path).unwrap(),
                [s, v, t] => load_edges(&mut builder, s, v, t, path, None).unwrap(),
                _ => {}
            }
        }
        let (mut graph, stats, schema) = builder.finish("fixture".into(), FORMAT);
        attach_attributes(&mut graph, attributes);
        assert_eq!(schema.node_labels["Person"], 2);
        assert_eq!(schema.node_labels["Message"], 1);
        assert_eq!(schema.node_labels["Company"], 1);
        assert_eq!(schema.node_labels["University"], 1);
        assert_eq!(schema.node_labels["Country"], 1);
        assert_eq!(schema.relationship_labels["KNOWS"], 2);
        assert_eq!(
            schema.relationship_labels["WORK_AT"], 1,
            "the unknown organisation is dangling"
        );
        assert_eq!(schema.relationship_labels["HAS_CREATOR"], 1);
        assert_eq!(schema.relationship_labels["IS_LOCATED_IN"], 1);
        assert_eq!(schema.node_labels["TagClass"], 1);
        assert_eq!(
            schema.relationship_labels["HAS_TYPE"], 1,
            "TagClass.id resolves against the tagclass file"
        );
        assert_eq!(stats.dangling_edges_dropped, 1);
        let person = graph
            .nodes
            .iter()
            .find(|n| n.id.as_str() == "Person:933")
            .unwrap();
        assert_eq!(person.props.get("sourceId"), Some(&Value::Int(933)));
        assert_eq!(
            person.props.get("id"),
            Some(&Value::String("Person:933".into())),
            "Grust mirrors the node identity as the id property"
        );
        let comment = graph
            .nodes
            .iter()
            .find(|n| n.id.as_str() == "Message:7")
            .unwrap();
        assert_eq!(comment.label.as_str(), "Message");
        assert_eq!(
            comment.props.get("kind"),
            Some(&Value::String("Comment".into()))
        );
        assert!(matches!(
            person.props.get("birthday"),
            Some(Value::DateTime(_))
        ));
        assert_eq!(
            person.props.get("speaks"),
            Some(&Value::StringArray(vec!["si".into(), "en".into()]))
        );
        let place = graph
            .nodes
            .iter()
            .find(|n| n.id.as_str() == "Country:933")
            .unwrap();
        assert!(
            place.props.get("type").is_none(),
            "the type column became the label"
        );
        let work = graph
            .edges
            .iter()
            .find(|e| e.label.as_str() == "WORK_AT")
            .unwrap();
        assert_eq!(
            (work.from.as_str(), work.to.as_str()),
            ("Person:933", "Company:1226")
        );
        assert_eq!(work.props.get("workFrom"), Some(&Value::Int(2013)));
        assert_eq!(schema.dominant_relationship.as_deref(), Some("KNOWS"));
    }
}
