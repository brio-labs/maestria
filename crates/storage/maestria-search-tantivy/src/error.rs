use maestria_ports::PortError;

/// Convert a Tantivy error into the storage port error type.
pub(crate) fn to_port_error(error: tantivy::TantivyError) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

/// Convert an I/O error into the storage port error type.
pub(super) fn to_io_port_error(error: std::io::Error) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}
