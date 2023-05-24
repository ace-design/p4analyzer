use crate::{
	base_abstractions::{Diagnostic, FileId},
	plugin_manager::{p4test::P4Test, plugin::DiagnosticProvider},
};

pub fn get_diagnostics(file_id: FileId, path: String, input: &str) -> Vec<Diagnostic> {
	P4Test::get_diagnostics(file_id, &path[7..], input.to_string())
}
