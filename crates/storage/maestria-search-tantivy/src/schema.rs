use maestria_ports::PortError;
use tantivy::schema::Schema;
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, TextFieldIndexing, TextOptions,
};

pub(super) const FIELD_KEY: &str = "chunk_key";
pub(super) const FIELD_ARTIFACT_ID: &str = "artifact_id";
pub(super) const FIELD_CHUNK_ID: &str = "chunk_id";
pub(super) const FIELD_TEXT: &str = "text";
pub(super) const FIELD_CARD_KEY: &str = "card_key";
pub(super) const FIELD_CARD_ARTIFACT_ID: &str = "card_artifact_id";
pub(super) const FIELD_CARD_ID: &str = "card_id";
pub(super) const FIELD_CARD_TITLE: &str = "card_title";
pub(super) const FIELD_CARD_BODY: &str = "card_body";
pub(super) const FIELD_PATH: &str = "path";
pub(super) const FIELD_FILENAME: &str = "filename";
pub(super) const FIELD_SYMBOL: &str = "symbol";
pub(super) const FIELD_CARD_PATH: &str = "card_path";
pub(super) const FIELD_CARD_FILENAME: &str = "card_filename";
pub(super) const FIELD_CARD_SYMBOL: &str = "card_symbol";

/// Resolved Tantivy fields for the canonical index schema.
pub(super) struct IndexFields {
    pub(crate) key: Field,
    pub(crate) artifact_id: Field,
    pub(crate) chunk_id: Field,
    pub(crate) text: Field,
    pub(crate) card_key: Field,
    pub(crate) card_artifact_id: Field,
    pub(crate) card_id: Field,
    pub(crate) card_title: Field,
    pub(crate) card_body: Field,
    pub(crate) path: Field,
    pub(crate) filename: Field,
    pub(crate) symbol: Field,
    pub(crate) card_path: Field,
    pub(crate) card_filename: Field,
    pub(crate) card_symbol: Field,
}

/// Resolve a named field against a schema.
pub(super) fn schema_field(schema: &Schema, name: &str) -> Result<Field, PortError> {
    schema
        .get_field(name)
        .map_err(|_| PortError::InternalContext {
            context: "missing Tantivy schema field",
            source: name.to_string(),
        })
}

pub(super) const CANONICAL_SCHEMA: &str = concat!(
    "chunk_key:string;artifact_id:u64;chunk_id:u64;text:text(default,freq_pos,stored);",
    "card_key:string;card_artifact_id:u64;card_id:u64;card_title:text(default,freq_pos,stored);",
    "card_body:text(default,freq_pos,stored);path:text(default,freq_pos,stored);",
    "filename:text(default,freq_pos,stored);symbol:text(default,freq_pos,stored);",
    "card_path:text(default,freq_pos,stored);card_filename:text(default,freq_pos,stored);",
    "card_symbol:text(default,freq_pos,stored)"
);

pub(super) fn schema() -> Schema {
    let mut builder = Schema::builder();
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer("default")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_options = TextOptions::default()
        .set_indexing_options(text_indexing.clone())
        .set_stored();

    builder.add_text_field(FIELD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_ARTIFACT_ID, INDEXED | FAST | STORED);
    builder.add_u64_field(FIELD_CHUNK_ID, INDEXED | FAST | STORED);
    builder.add_text_field(FIELD_TEXT, text_options.clone());
    builder.add_text_field(FIELD_CARD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_CARD_ARTIFACT_ID, INDEXED | FAST | STORED);
    builder.add_u64_field(FIELD_CARD_ID, INDEXED | FAST | STORED);
    builder.add_text_field(FIELD_CARD_TITLE, text_options.clone());
    builder.add_text_field(FIELD_CARD_BODY, text_options.clone());
    builder.add_text_field(FIELD_PATH, text_options.clone());
    builder.add_text_field(FIELD_FILENAME, text_options.clone());
    builder.add_text_field(FIELD_SYMBOL, text_options.clone());
    builder.add_text_field(FIELD_CARD_PATH, text_options.clone());
    builder.add_text_field(FIELD_CARD_FILENAME, text_options.clone());
    builder.add_text_field(FIELD_CARD_SYMBOL, text_options);
    builder.build()
}

pub(super) fn supports_filtered_queries(schema: &Schema) -> bool {
    [FIELD_CHUNK_ID, FIELD_CARD_ID].into_iter().all(|name| {
        schema
            .get_field(name)
            .ok()
            .is_some_and(|field| schema.get_field_entry(field).is_indexed())
    })
}

pub(super) fn load_fields(schema: Schema) -> Result<IndexFields, PortError> {
    Ok(IndexFields {
        key: schema_field(&schema, FIELD_KEY)?,
        artifact_id: schema_field(&schema, FIELD_ARTIFACT_ID)?,
        chunk_id: schema_field(&schema, FIELD_CHUNK_ID)?,
        text: schema_field(&schema, FIELD_TEXT)?,
        card_key: schema_field(&schema, FIELD_CARD_KEY)?,
        card_artifact_id: schema_field(&schema, FIELD_CARD_ARTIFACT_ID)?,
        card_id: schema_field(&schema, FIELD_CARD_ID)?,
        card_title: schema_field(&schema, FIELD_CARD_TITLE)?,
        card_body: schema_field(&schema, FIELD_CARD_BODY)?,
        path: schema_field(&schema, FIELD_PATH)?,
        filename: schema_field(&schema, FIELD_FILENAME)?,
        symbol: schema_field(&schema, FIELD_SYMBOL)?,
        card_path: schema_field(&schema, FIELD_CARD_PATH)?,
        card_filename: schema_field(&schema, FIELD_CARD_FILENAME)?,
        card_symbol: schema_field(&schema, FIELD_CARD_SYMBOL)?,
    })
}
