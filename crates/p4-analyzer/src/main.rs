mod cli;
mod commands;
mod stdio;

use cancellation::{CancellationToken, CancellationTokenSource};
use cli::flags::{P4Analyzer, P4AnalyzerCmd};
use commands::lsp_server::LspServerCommand;
use commands::{Command, CommandInvocationError};
use std::{
	process,
	sync::{
		atomic::{AtomicU8, Ordering},
		Arc,
	},
};

/// Entry point for the P4 Analyzer.
#[tokio::main]
pub async fn main() {
	match P4Analyzer::from_env() {
		Ok(cmd) => match cmd.subcommand {
			P4AnalyzerCmd::Server(config) => {
				run_cancellable_command(&LspServerCommand::new(config)).await;
			}
		},
		Err(err) => {
			println!();
			println!("{}", err);
			println!();
		}
	}
}

/// Executes a command.
///
/// The supplied command will be invoked with a [`CancellationToken`] that is canceled upon receiving a 'Ctrl-C' signal (if
/// it is supported by the platform).
async fn run_cancellable_command(cmd: &dyn Command) {
	let count = Arc::new(AtomicU8::new(0));

	let cancellation_source = CancellationTokenSource::new();
	let cancellation_token = cancellation_source.token().clone();

	ctrlc::set_handler(move || {
		let prev_count = count.fetch_add(1, Ordering::Relaxed);

		if prev_count == 0 {
			eprintln!();
			eprintln!("(To forcibly exit, press 'Ctrl+C' again)");

			cancellation_source.cancel();
		}

		if prev_count > 0 {
			process::exit(-1);
		}
	})
	.expect("'Ctrl-C' handling is not available for this platform.");

	run_command_with_cancel_token(cmd, cancellation_token).await;
}

/// Executes a command, writing any errors to the error console.
///
/// The supplied command can also be cancelled via the supplied [`CancellationToken`].
async fn run_command_with_cancel_token(cmd: &dyn Command, cancel_token: Arc<CancellationToken>) {
	match cmd.run(cancel_token).await {
		Ok(_) => {}
		Err(err) => match err {
			CommandInvocationError::Cancelled => println!("{}", err),
			_ => eprintln!("{}", err),
		},
	}
}
