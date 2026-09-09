//! ICIJ Offshore Leaks database, the CSV zip (`full-oldb.LATEST.zip`): five
//! node files (`nodes-entities.csv`, `nodes-officers.csv`,
//! `nodes-intermediaries.csv`, `nodes-addresses.csv`, `nodes-others.csv`)
//! and `relationships.csv`. Node labels are the file's kind (Entity,
//! Officer, Intermediary, Address, Other); `node_id` is unique across the
//! files and is the node id as well as the `sourceId` property (Grust
//! mirrors a node's identity as its `id` property, so the source column
//! keeps its own name). Relationship
//! types are `rel_type` in upper snake case (`OFFICER_OF`,
//! `REGISTERED_ADDRESS`), with `link`, `status`, `start_date`, `end_date`
//! and `sourceID` as properties. Every other column is a string property;
//! empty cells are absent. The zip is read in place.

use std::io::Read;
use std::path::Path;

use grust::{Graph, Props};

use super::LoadStats;
use super::typed::{
    DatasetSchema, TypedGraphBuilder, integer_columns, relationship_label, typed_value,
};

pub const FORMAT: &str = "icij-offshore-leaks";
const NODE_FILES: &[(&str, &str)] = &[
    ("nodes-entities.csv", "Entity"),
    ("nodes-officers.csv", "Officer"),
    ("nodes-intermediaries.csv", "Intermediary"),
    ("nodes-addresses.csv", "Address"),
    ("nodes-others.csv", "Other"),
];
const RELATIONSHIPS: &str = "relationships.csv";

pub fn load(
    archive: &Path,
    limit: Option<usize>,
) -> std::io::Result<(Graph, LoadStats, DatasetSchema)> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(std::io::Error::other)?;
    let mut builder = TypedGraphBuilder::new(limit);
    for (name, label) in NODE_FILES {
        let entry = zip.by_name(name).map_err(std::io::Error::other)?;
        load_nodes(&mut builder, label, entry)?;
    }
    let entry = zip.by_name(RELATIONSHIPS).map_err(std::io::Error::other)?;
    load_relationships(&mut builder, entry)?;
    Ok(builder.finish(archive.display().to_string(), FORMAT))
}

fn reader(source: impl Read) -> csv::Reader<impl Read> {
    csv::ReaderBuilder::new().flexible(true).from_reader(source)
}

fn load_nodes(
    builder: &mut TypedGraphBuilder,
    label: &str,
    source: impl Read,
) -> std::io::Result<()> {
    let mut reader = reader(source);
    let headers = reader.headers()?.clone();
    let records: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;
    let int_columns = integer_columns(&headers, &records);
    for record in records {
        builder.lines += 1;
        let Some(id) = record.get(0).filter(|id| !id.is_empty()) else {
            continue;
        };
        let mut props = Props::new();
        for (i, (column, cell)) in headers.iter().zip(record.iter()).enumerate() {
            let column = if column == "node_id" {
                "sourceId"
            } else {
                column
            };
            if let Some(value) = typed_value(column, cell, int_columns[i], &[]) {
                props.insert(column.to_string(), value);
            }
        }
        builder.node(label, id.to_string(), props);
    }
    Ok(())
}

fn load_relationships(builder: &mut TypedGraphBuilder, source: impl Read) -> std::io::Result<()> {
    let mut reader = reader(source);
    let headers = reader.headers()?.clone();
    let column = |name: &str| headers.iter().position(|h| h == name);
    let (Some(start), Some(end), Some(kind)) = (
        column("node_id_start"),
        column("node_id_end"),
        column("rel_type"),
    ) else {
        return Err(std::io::Error::other(
            "relationships.csv lacks node_id_start, node_id_end or rel_type",
        ));
    };
    let records: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;
    let int_columns = integer_columns(&headers, &records);
    for record in records {
        builder.lines += 1;
        let (Some(from), Some(to), Some(rel_type)) =
            (record.get(start), record.get(end), record.get(kind))
        else {
            continue;
        };
        let label = relationship_label(rel_type);
        let mut props = Props::new();
        for (index, (name, cell)) in headers.iter().zip(record.iter()).enumerate() {
            if index == start || index == end || index == kind {
                continue;
            }
            if let Some(value) = typed_value(name, cell, int_columns[index], &[]) {
                props.insert(name.to_string(), value);
            }
        }
        builder.edge(&label, from, to, props);
        if builder.full() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grust::Value;

    #[test]
    fn typed_nodes_and_relationships_from_csv_text() {
        let entities = "node_id,name,jurisdiction,incorporation_date,status\n10000001,\"TIANSHENG INDUSTRY AND TRADING CO., LTD.\",SAM,23-MAR-2006,Defaulted\n";
        let officers =
            "node_id,name,countries,sourceID\n12000001,KIM SOO IN,South Korea,Panama Papers\n";
        let addresses = "node_id,address,countries\n24000001,\"ANNEX FREDERICK & SHIRLEY STS, NASSAU\",Bahamas\n";
        let relationships = "node_id_start,node_id_end,rel_type,link,status,start_date,end_date,sourceID\n12000001,10000001,officer_of,shareholder of,,,,Panama Papers\n10000001,24000001,registered_address,registered address,,,,Panama Papers\n12000001,10000001,officer_of,shareholder of,,,,Panama Papers\n12000001,99,officer_of,director of,,,,Panama Papers\n";
        let mut builder = TypedGraphBuilder::new(None);
        load_nodes(&mut builder, "Entity", entities.as_bytes()).unwrap();
        load_nodes(&mut builder, "Officer", officers.as_bytes()).unwrap();
        load_nodes(&mut builder, "Address", addresses.as_bytes()).unwrap();
        load_relationships(&mut builder, relationships.as_bytes()).unwrap();
        let (graph, stats, schema) = builder.finish("fixture".into(), FORMAT);
        assert_eq!(schema.node_labels["Entity"], 1);
        assert_eq!(schema.relationship_labels["OFFICER_OF"], 1);
        assert_eq!(schema.relationship_labels["REGISTERED_ADDRESS"], 1);
        assert_eq!(
            (stats.duplicate_edges_dropped, stats.dangling_edges_dropped),
            (1, 1)
        );
        let entity = graph
            .nodes
            .iter()
            .find(|n| n.id.as_str() == "10000001")
            .unwrap();
        assert_eq!(entity.props.get("sourceId"), Some(&Value::Int(10000001)));
        assert_eq!(
            entity.props.get("id"),
            Some(&Value::String("10000001".into()))
        );
        assert_eq!(
            entity.props.get("name"),
            Some(&Value::String(
                "TIANSHENG INDUSTRY AND TRADING CO., LTD.".into()
            ))
        );
        assert_eq!(
            entity.props.get("incorporation_date"),
            Some(&Value::String("23-MAR-2006".into()))
        );
        let officer_of = graph
            .edges
            .iter()
            .find(|e| e.label.as_str() == "OFFICER_OF")
            .unwrap();
        assert_eq!(
            officer_of.props.get("link"),
            Some(&Value::String("shareholder of".into()))
        );
        assert!(
            officer_of.props.get("status").is_none(),
            "empty cells are absent"
        );
    }
}
