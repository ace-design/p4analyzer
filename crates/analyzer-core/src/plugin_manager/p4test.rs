use crate::base_abstractions::{Diagnostic, FileId, Severity};
use regex::Regex;
use std::{
	fs::File,
	io::prelude::*,
	ops::Range,
	path::PathBuf,
	process::{Command, Stdio},
};

use crate::plugin_manager::plugin::{DiagnosticProvider, Plugin};

pub struct P4Test;

impl Plugin for P4Test {}

impl DiagnosticProvider for P4Test {
	fn get_diagnostics(file: FileId, path: &str, content: String) -> Vec<Diagnostic> {
		let mut file_ = File::create("/tmp/foo.txt").unwrap();
		file_.write_all(b"Hello World").unwrap();
		let output = get_p4test_output(path);

		file_.write_all(output.clone().unwrap_or(String::from("ERROR")).as_bytes()).unwrap();

		if let Some(output) = output {
			parse_output(output, file, path, &content).unwrap_or_default()
		} else {
			vec![]
		}
	}
}

fn get_p4test_output(path: &str) -> Option<String> {
	let include_path = PathBuf::from("/home/alex/Documents/University/Master/p4c/p4include");
	let p4test_path = PathBuf::from("/home/alex/.local/bin/p4c_backend_p4test");

	let command_result = Command::new(p4test_path)
		.arg(path)
		.arg("-I")
		.arg(include_path.as_os_str())
		.stdin(Stdio::piped())
		.stderr(Stdio::piped())
		.output()
		.ok()?;

	String::from_utf8(command_result.stderr).ok()
}

fn parse_output(message: String, file_id: FileId, path: &str, text: &str) -> Option<Vec<Diagnostic>> {
	// Parse and remove line number
	let line_nb_re = Regex::new(format!("{}{}", path, r"\((\d+)\):?").as_str()).unwrap();
	let captures = line_nb_re.captures(&message)?;
	let line_nb = captures.get(1)?.as_str().parse::<u32>().ok()? - 1;
	let current_msg = line_nb_re.replace(&message, "");

	let kind_re = Regex::new(r"\[--W(.*)=(.*)\]").unwrap();
	let captures = kind_re.captures(&current_msg);

	// Parse and remove severity and kind
	let (severity, _kind) = if let Some(captures) = captures {
		let severity_capture = captures.get(1);
		let severity = if let Some(cap) = severity_capture {
			match cap.as_str() {
				"error" => Severity::Error,
				"warn" => Severity::Warning,
				_ => Severity::Error,
			}
		} else {
			Severity::Error
		};

		let kind_cap = captures.get(2);
		let kind = if let Some(cap) = kind_cap { cap.as_str() } else { "" };

		(severity, kind)
	} else {
		(Severity::Error, "")
	};
	let current_msg = kind_re.replace(&current_msg, "");

	// Make and return diagnostic
	let lines: Vec<&str> = current_msg.trim().lines().collect();

	let diag_msg = lines[0].replace("error:", "").replace("warning:", "");
	let diag_range = get_range(line_nb, lines[2], text);

	Some(vec![Diagnostic {
		file: file_id,
		severity: severity,
		location: diag_range,
		message: diag_msg.trim().to_string(),
	}])
}

fn get_range(line_nb: u32, arrows: &str, text: &str) -> Range<usize> {
	let mut start: u32 = 0;

	for char in arrows.chars() {
		if char == ' ' {
			start += 1;
		} else {
			break;
		}
	}

	let mut total_bytes = 0;
	let lines = &text.lines().collect::<Vec<&str>>()[..(line_nb as usize)];

	for line in lines {
		total_bytes += line.len() + 1; // WARNING: Could break if line break char is not 1 byte
	}
	let start_byte = total_bytes + start as usize;

	start_byte..(start_byte + arrows.len())
}
