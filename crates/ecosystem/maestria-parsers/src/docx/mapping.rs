use maestria_ports::PortError;

use super::{MAX_DOCX_PARAGRAPHS, MAX_DOCX_TEXT_BYTES, invalid};

pub(super) struct TextBlock {
    pub(super) text: String,
    pub(super) start_paragraph: usize,
    pub(super) end_paragraph: usize,
}

#[derive(Default)]
struct TableRow {
    start_paragraph: Option<usize>,
    end_paragraph: Option<usize>,
    cells: Vec<String>,
}

#[derive(Default)]
struct Paragraph {
    number: usize,
    text: String,
}

#[derive(Default)]
pub(super) struct BlockMapper {
    blocks: Vec<TextBlock>,
    table_rows: Vec<TableRow>,
    cells: Vec<String>,
    paragraph: Option<Paragraph>,
    paragraph_count: usize,
    decoded_text_bytes: usize,
    output_text_bytes: usize,
}

impl BlockMapper {
    pub(super) fn begin_paragraph(&mut self) -> Result<(), PortError> {
        if self.paragraph.is_some() {
            return Err(invalid("DOCX document contains nested paragraphs"));
        }
        self.paragraph_count = self
            .paragraph_count
            .checked_add(1)
            .ok_or_else(|| invalid("DOCX paragraph count overflow"))?;
        if self.paragraph_count > MAX_DOCX_PARAGRAPHS {
            return Err(invalid(format!(
                "DOCX paragraph count exceeds the limit of {MAX_DOCX_PARAGRAPHS}"
            )));
        }
        for row in &mut self.table_rows {
            row.start_paragraph.get_or_insert(self.paragraph_count);
            row.end_paragraph = Some(self.paragraph_count);
        }
        self.paragraph = Some(Paragraph {
            number: self.paragraph_count,
            text: String::new(),
        });
        Ok(())
    }

    pub(super) fn append_text(&mut self, fragment: &str) -> Result<(), PortError> {
        let Some(paragraph) = self.paragraph.as_mut() else {
            return Ok(());
        };
        self.decoded_text_bytes = self
            .decoded_text_bytes
            .checked_add(fragment.len())
            .ok_or_else(|| invalid("DOCX extracted text size overflow"))?;
        if self.decoded_text_bytes > MAX_DOCX_TEXT_BYTES {
            return Err(invalid(format!(
                "DOCX extracted text exceeds the {MAX_DOCX_TEXT_BYTES} byte limit"
            )));
        }
        paragraph.text.push_str(fragment);
        Ok(())
    }

    pub(super) fn finish_paragraph(&mut self) -> Result<(), PortError> {
        let paragraph = self
            .paragraph
            .take()
            .ok_or_else(|| invalid("DOCX paragraph end has no matching start"))?;
        let text = paragraph.text.trim();
        if text.is_empty() {
            return Ok(());
        }
        if let Some(cell) = self.cells.last_mut() {
            append_cell_text(cell, text)?;
        } else if let Some(row) = self.table_rows.last_mut() {
            row.cells.push(text.to_owned());
        } else {
            self.push_block(text.to_owned(), paragraph.number, paragraph.number)?;
        }
        Ok(())
    }

    pub(super) fn start_row(&mut self) {
        self.table_rows.push(TableRow::default());
    }

    pub(super) fn start_cell(&mut self) {
        self.cells.push(String::new());
    }

    pub(super) fn finish_cell(&mut self) -> Result<(), PortError> {
        let cell = match self.cells.pop() {
            Some(cell) => cell,
            None => return Err(invalid("DOCX table cell is not balanced")),
        };
        let row = self
            .table_rows
            .last_mut()
            .ok_or_else(|| invalid("DOCX table row is not balanced"))?;
        row.cells.push(cell);
        Ok(())
    }

    pub(super) fn finish_row(&mut self) -> Result<(), PortError> {
        let row = self
            .table_rows
            .pop()
            .ok_or_else(|| invalid("DOCX table row end has no matching start"))?;
        let Some(start_paragraph) = row.start_paragraph else {
            return Ok(());
        };
        let Some(end_paragraph) = row.end_paragraph else {
            return Err(invalid("DOCX table row has no paragraph end"));
        };
        let text = row
            .cells
            .iter()
            .map(|cell| cell.trim())
            .collect::<Vec<_>>()
            .join(" | ");
        if text.trim().is_empty() {
            return Ok(());
        }
        if let Some(parent_cell) = self.cells.last_mut() {
            append_cell_text(parent_cell, &text)?;
        } else {
            self.push_block(text, start_paragraph, end_paragraph)?;
        }
        Ok(())
    }

    pub(super) fn has_open_paragraph(&self) -> bool {
        self.paragraph.is_some()
    }

    pub(super) fn has_open_row(&self) -> bool {
        !self.table_rows.is_empty()
    }

    pub(super) fn has_open_content(&self) -> bool {
        self.paragraph.is_some() || !self.table_rows.is_empty() || !self.cells.is_empty()
    }

    pub(super) fn into_blocks(self) -> Vec<TextBlock> {
        self.blocks
    }

    fn push_block(
        &mut self,
        text: String,
        start_paragraph: usize,
        end_paragraph: usize,
    ) -> Result<(), PortError> {
        if self.blocks.len() >= MAX_DOCX_PARAGRAPHS {
            return Err(invalid(format!(
                "DOCX extracted block count exceeds the limit of {MAX_DOCX_PARAGRAPHS}"
            )));
        }
        self.output_text_bytes = self
            .output_text_bytes
            .checked_add(text.len())
            .and_then(|bytes| bytes.checked_add(1))
            .ok_or_else(|| invalid("DOCX output text size overflow"))?;
        if self.output_text_bytes > MAX_DOCX_TEXT_BYTES {
            return Err(invalid(format!(
                "DOCX output text exceeds the {MAX_DOCX_TEXT_BYTES} byte limit"
            )));
        }
        self.blocks.push(TextBlock {
            text,
            start_paragraph,
            end_paragraph,
        });
        Ok(())
    }
}

fn append_cell_text(cell: &mut String, text: &str) -> Result<(), PortError> {
    let additional = text.len() + usize::from(!cell.is_empty());
    if cell.len().saturating_add(additional) > MAX_DOCX_TEXT_BYTES {
        return Err(invalid(format!(
            "DOCX table text exceeds the {MAX_DOCX_TEXT_BYTES} byte limit"
        )));
    }
    if !cell.is_empty() {
        cell.push('\n');
    }
    cell.push_str(text);
    Ok(())
}
