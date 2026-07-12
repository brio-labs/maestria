use maestria_ports::PortError;
use tantivy::schema::Schema;
use tantivy::schema::{FAST, IndexRecordOption, STORED, STRING, TextFieldIndexing, TextOptions};

use super::{
    FIELD_ARTIFACT_ID, FIELD_CARD_ARTIFACT_ID, FIELD_CARD_BODY, FIELD_CARD_ID, FIELD_CARD_KEY,
    FIELD_CARD_TITLE, FIELD_CHUNK_ID, FIELD_KEY, FIELD_TEXT, IndexFields, schema_field,
};

pub(super) fn schema() -> Schema {
    let mut builder = Schema::builder();
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer("default")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_options = TextOptions::default()
        .set_indexing_options(text_indexing.clone())
        .set_stored();

    builder.add_text_field(FIELD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_ARTIFACT_ID, FAST | STORED);
    builder.add_u64_field(FIELD_CHUNK_ID, FAST | STORED);
    builder.add_text_field(FIELD_TEXT, text_options.clone());
    builder.add_text_field(FIELD_CARD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_CARD_ARTIFACT_ID, FAST | STORED);
    builder.add_u64_field(FIELD_CARD_ID, FAST | STORED);
    builder.add_text_field(FIELD_CARD_TITLE, text_options.clone());
    builder.add_text_field(FIELD_CARD_BODY, text_options);
    builder.build()
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
    })
}
