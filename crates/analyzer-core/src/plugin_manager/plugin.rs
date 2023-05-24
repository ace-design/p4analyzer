use crate::base_abstractions::{Diagnostic, FileId};

pub trait Plugin: DiagnosticProvider {}

pub trait DiagnosticProvider {
	fn get_diagnostics(_file: FileId, _path: &str, _file_content: String) -> Vec<Diagnostic> { vec![] }
}
