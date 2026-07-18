use maestria_ports::PortError;
use tantivy::schema::Schema;
use tantivy::schema::{
    FAST, INDEXED, IndexRecordOption, STORED, STRING, TextFieldIndexing, TextOptions,
};

use super::{
    FIELD_ARTIFACT_ID, FIELD_CARD_ARTIFACT_ID, FIELD_CARD_BODY, FIELD_CARD_FILENAME, FIELD_CARD_ID,
    FIELD_CARD_KEY, FIELD_CARD_PATH, FIELD_CARD_SYMBOL, FIELD_CARD_TITLE, FIELD_CHUNK_ID,
    FIELD_FILENAME, FIELD_KEY, FIELD_PATH, FIELD_SYMBOL, FIELD_TEXT, IndexFields, schema_field,
};

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
