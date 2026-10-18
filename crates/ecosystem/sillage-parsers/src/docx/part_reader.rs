use std::io::{Cursor, Read};

use zip::ZipArchive;

use super::invalid;

pub(super) fn read_part(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    expected_size: usize,
    maximum_size: usize,
) -> Result<Vec<u8>, sillage_ports::PortError> {
    let part = archive
        .by_name(name)
        .map_err(|error| invalid(format!("read DOCX part {name}: {error}")))?;
    if part.size() != expected_size as u64 || expected_size > maximum_size {
        return Err(invalid(format!("DOCX part {name} exceeds its size limit")));
    }
    let mut output = Vec::with_capacity(expected_size);
    part.take(maximum_size as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| invalid(format!("decompress DOCX part {name}: {error}")))?;
    if output.len() != expected_size {
        return Err(invalid(format!(
            "DOCX part {name} decompressed size does not match its ZIP metadata"
        )));
    }
    Ok(output)
}
