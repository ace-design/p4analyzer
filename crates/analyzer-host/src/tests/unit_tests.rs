mod fsm_tests {
  use std::sync::Arc;
	
	use crate::fsm::LspProtocolMachine;
  use crate::json_rpc::message::Message;
  use crate::lsp::request::RequestManager;
  use crate::lsp::state::LspServerState;
  use crate::{fs::LspEnumerableFileSystem};

	#[test]
	fn test_is_active() {
		let rm = RequestManager::new(async_channel::unbounded::<Message>());
		let mut lsp = LspProtocolMachine::new(None, rm.clone(),
													Arc::new(Box::new(LspEnumerableFileSystem::new(rm.clone()))));
		assert_eq!(lsp.is_active(), true);

		let mut _output = lsp.set_state("initialize".to_string());
		assert_eq!(lsp.is_active(), true);

		_output = lsp.set_state("initialize".to_string());
		assert_eq!(lsp.is_active(), true);

		_output = lsp.set_state("initialized".to_string());
		assert_eq!(lsp.is_active(), true);

		_output = lsp.set_state("shutdown".to_string());
		assert_eq!(lsp.is_active(), true);

		_output = lsp.set_state("exit".to_string());
		assert_eq!(lsp.is_active(), false);
	}
	#[test]
	fn test_process_message() {
		let rm = RequestManager::new(async_channel::unbounded::<Message>());
		let mut lsp = LspProtocolMachine::new(None, rm.clone(), 
													Arc::new(Box::new(LspEnumerableFileSystem::new(rm.clone()))));
		assert_eq!(lsp.current_state(), LspServerState::ActiveUninitialized);

		let mut output = lsp.set_state("initialize".to_string());
		assert!(output.is_ok());
		assert_eq!(lsp.current_state(), LspServerState::Initializing);

		output = lsp.set_state("initialize".to_string());
		assert!(output.is_ok());
		assert_eq!(lsp.current_state(), LspServerState::Initializing);

		output = lsp.set_state("initialized".to_string());
		assert!(output.is_ok());
		assert_eq!(lsp.current_state(), LspServerState::ActiveInitialized);

		output = lsp.set_state("initialized".to_string());
		assert!(output.is_err());
		assert_eq!(lsp.current_state(), LspServerState::ActiveInitialized);

		output = lsp.set_state("shutdown".to_string());
		assert!(output.is_ok());
		assert_eq!(lsp.current_state(), LspServerState::ShuttingDown);

		output = lsp.set_state("exit".to_string());
		assert!(output.is_ok());
		assert_eq!(lsp.current_state(), LspServerState::Stopped);
	}
}