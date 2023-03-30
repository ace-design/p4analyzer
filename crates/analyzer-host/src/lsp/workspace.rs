use core::fmt::Debug;
use std::{
	sync::{Arc, RwLock, Mutex, RwLockWriteGuard},
	collections::{HashMap, hash_map::{Iter, IntoIter, Entry}}, fmt::{Formatter, Display, Result as FmtResult},
	task::Poll
};

use analyzer_abstractions::{
	lsp_types::{WorkspaceFolder, Url, TextDocumentIdentifier},
	fs::AnyEnumerableFileSystem,
	tracing::{error, info}, futures_extensions::async_extensions::AsyncPool
};
use analyzer_abstractions::futures_extensions::FutureCompletionSource;
use async_channel::{Sender, Receiver};
use thiserror::Error;

use super::progress::ProgressManager;

/// Manages a collection of workspaces opened by an LSP compliant host.
#[derive(Clone)]
pub(crate) struct WorkspaceManager {
	has_workspaces: bool,
	workspaces: HashMap<Url, Arc<Workspace>>,
}

impl WorkspaceManager {
	/// Initializes a new [`WorkspaceManager`] instance.
	///
	/// If `workspace_folders` is [`None`], then a root workspace folder will be used by default.
	pub fn new(file_system: Arc<AnyEnumerableFileSystem>, workspace_folders: Option<Vec<WorkspaceFolder>>) -> Self {
		fn to_workspace(
			file_system: Arc<AnyEnumerableFileSystem>,
			workspace_folder: WorkspaceFolder
		) -> (Url, Arc<Workspace>)
		{
			(workspace_folder.uri.clone(), Arc::new(Workspace::new(file_system, workspace_folder)))
		}

		let (has_workspaces, workspace_folders) = workspace_folders
			.map_or_else(
				|| { (false, vec![WorkspaceFolder{ name: "<*>".to_string(), uri: Url::parse("file:///").unwrap() }]) },
				|folders| { (true, folders) });

		Self {
			has_workspaces,
			workspaces: workspace_folders.into_iter().map(|wf| to_workspace(file_system.clone(), wf)).collect()
		}
	}

	/// Returns `true` if the [`WorkspaceManager`] was initialized with workspace folders; otherwise `false`.
	pub fn has_workspaces(&self) -> bool {
		self.has_workspaces
	}

	/// Retrieves a file from a workspace.
	///
	/// [`WorkspaceManager::get_file`] will always return a [`File`] for `uri`. It does this because requests for
	/// files may be made in contexts in which no workspace folders were opened. If this is the case, then the file will
	/// be retrieved relative to the 'catch-all' workspace which is not indexed.
	///
	/// The overall state of the file can be determined from its [`File::get_compiled_unit`] method which will
	/// inform its final state.
	pub fn get_file(&self, uri: Url) -> Arc<File> {
		fn is_descendant_path(base: &Url, target: &Url) -> bool {
			if let Some(relative) = base.make_relative(target) {
				return !relative.starts_with("..");
			}

			false
		}

		// If not initialized with any workspace folders, then the path should always be a descendant of the
		// 'catch-all' workspace.
		match (&self.workspaces).into_iter().find(|(workspace_uri, _)| is_descendant_path(&workspace_uri, &uri)) {
			Some((_, workspace)) => workspace.get_file(uri),
			None => {
				error!(file_uri = uri.as_str(), "Failed to locate a workspace for a given file.");
				unreachable!("failed to locate a workspace");
			}
		}
	}

	/// Asynchronously indexes the contents of each [`Workspace`].
	///
	/// Returns immediately if the the [`WorkspaceManager`] was not initialized with workspace folders.
	pub async fn index(&self, progress: &ProgressManager) {
		if !self.has_workspaces() {
			return; // Do nothing if there are no workspace folders.
		}

		let progress = progress.begin("Indexing").await.unwrap();

		for (_, workspace) in (&self.workspaces).into_iter() {
			progress.report(&format!("{}", workspace)).await.unwrap();

			workspace.index().await;
		}

		progress.end(None).await.unwrap();
	}
}

impl IntoIterator for WorkspaceManager {
	type Item = (Url, Arc<Workspace>);
	type IntoIter = IntoIter<Url, Arc<Workspace>>;

	/// Creates a consuming iterator of [`Workspace`].
	fn into_iter(self) -> Self::IntoIter {
		self.workspaces.into_iter()
	}
}

impl<'a> IntoIterator for &'a WorkspaceManager {
	type Item = (&'a Url, &'a Arc<Workspace>);
	type IntoIter = Iter<'a, Url, Arc<Workspace>>;

	/// Creates a consuming iterator of &[`Workspace`].
	fn into_iter(self) -> Self::IntoIter {
		self.workspaces.iter()
	}
}

/// Encapsulates a collection of related files opened as part of a set managed by an LSP compliant host.
#[derive(Clone)]
pub(crate) struct Workspace {
	file_system: Arc<AnyEnumerableFileSystem>,
	workspace_folder: WorkspaceFolder,
	files: Arc<RwLock<HashMap<Url, Arc<File>>>>,
	parse_sender: Sender<Arc<File>>
}

impl Workspace {
	/// Initializes a new [`Workspace`].
	pub fn new(file_system: Arc<AnyEnumerableFileSystem>, workspace_folder: WorkspaceFolder) -> Self {
		let (sender, receiver) = async_channel::unbounded::<Arc<File>>();

		AsyncPool::spawn_work(background_parse(receiver, file_system.clone()));

		Self {
			file_system,
			workspace_folder,
			files: Arc::new(RwLock::new(HashMap::new())),
			parse_sender: sender
		}
	}

	/// Gets the URL of the current [`Workspace`].
	pub fn uri(&self) -> Url {
		self.workspace_folder.uri.clone()
	}

	/// Gets the name of the current [`Workspace`].
	pub fn name(&self) -> &str {
		self.workspace_folder.name.as_str()
	}

	/// Look up and retrieve a file from the workspace.
	///
	/// The [`File`] will be created if it is not present in the current workspace.
	pub fn get_file(&self, uri: Url) -> Arc<File> {
		let mut files = self.files.write().unwrap();
		let workspace_uri = self.uri();
		let new_uri = uri.clone();

		match files.entry(uri) {
			Entry::Occupied(entry) => entry.get().clone(),
			Entry::Vacant(entry) => {
				info!(workspace_uri = workspace_uri.as_str(), file_uri = new_uri.as_str(), "Missing file entry in workspace'{}'.", self.name());

				let new_document_identifier = TextDocumentIdentifier { uri: new_uri };
				let new_file = Arc::new(File::new(new_document_identifier.clone()));

				entry.insert(new_file.clone());

				self.parse_sender.send_blocking(new_file.clone()).unwrap();

				new_file
			}
		}
	}

	pub async fn index(&self) {
		fn write_files(s: &Workspace, document_identifiers: &Vec<TextDocumentIdentifier>) {
			let mut files = s.files.write().unwrap();

			for document_identifier in document_identifiers.into_iter() {
				let new_file = Arc::new(File::new(document_identifier.clone()));

				files.insert(document_identifier.uri.clone(), new_file.clone());

				s.parse_sender.send_blocking(new_file.clone()).unwrap();
			}
		}

		let document_identifiers = self.file_system.enumerate_folder(self.uri()).await;

		if document_identifiers.len() == 0 {
			return;
		}

		write_files(self, &document_identifiers);
	}
}

impl Display for Workspace {
	/// Formats a [`Workspace`] using the given formatter.
	fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
		write!(formatter, "[{}]({})", self.workspace_folder.name, self.workspace_folder.uri)?;

		Ok(())
	}
}

#[derive(Error, Debug, PartialEq, Eq, Clone, Copy)]
pub enum IndexError {
	#[error("An unexpected error occurred during file indexing.")]
	Unexpected
}

type CompiledUnit = ();

#[derive(Clone)]
struct FileState<T: Clone = CompiledUnit> {
	buffer: Option<String>,
	compiled_unit: FutureCompletionSource<Box<T>, IndexError>
}

#[derive(Clone)]
pub(crate) struct File {
	document_identifier: TextDocumentIdentifier,
	state: Arc<RwLock<FileState>>
}

impl File {
	pub fn new(document_identifier: TextDocumentIdentifier) -> Self {
		Self {
			document_identifier,
			state: Arc::new(RwLock::new(FileState {
				buffer: None,
				compiled_unit: FutureCompletionSource::<Box<CompiledUnit>, IndexError>::new()
			}))
		}
	}

	/// Returns `true` if the current file has a buffer open and under the control of an LSP compliant host.
	pub fn is_open_in_ide(&self) -> bool {
		let state = self.state.read().unwrap();

		state.buffer == None
	}

	/// Returns the current buffer.
	///
	/// Returns [`None`] if the file has no buffer (indicating that the file is not open).
	pub fn current_buffer(&self) -> Option<String> {
		let state = self.state.read().unwrap();

		state.buffer.clone()
	}

	pub async fn get_compiled_unit(&self) -> Result<CompiledUnit, IndexError> {
		let state = self.state.read().unwrap();

		match state.compiled_unit.future().await {
			Ok(boxed_value) => {
				Ok(*boxed_value.clone())
			},
			Err(err) => Err(err)
		}
	}

	fn set_compiled_unit(&self, compiled_unit: CompiledUnit, state: Option<RwLockWriteGuard<FileState<CompiledUnit>>>) {
		let mut state = state.unwrap_or_else(|| self.state.write().unwrap());

		if let Poll::Ready(result) = state.compiled_unit.state() {
			match result {
				Ok(mut boxed_value) => *boxed_value = compiled_unit,
				Err(_) => state.compiled_unit = FutureCompletionSource::<Box<CompiledUnit>, IndexError>::new_with_value(Box::new(compiled_unit))
			}
		}
		else {
			state.compiled_unit.set_value(Box::new(compiled_unit)).unwrap();
		}
	}

	pub fn open_or_change_buffer(&self, buffer: String, compiled_unit: CompiledUnit) {
		let mut state = self.state.write().unwrap();

		state.buffer.replace(buffer);

		self.set_compiled_unit(compiled_unit, Some(state)); // Use the writable state that we already have.
	}

	pub fn close_buffer(&self) {
		let mut state = self.state.write().unwrap();

		state.buffer = None;
	}
}

impl Display for File {
	/// Formats a [`Workspace`] using the given formatter.
	fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
		write!(formatter, "({})", self.document_identifier.uri)?;

		Ok(())
	}
}

async fn background_parse(receiver: Receiver<Arc<File>>, file_system: Arc<AnyEnumerableFileSystem>) {
	loop {
		match receiver.recv().await {
			Ok(file) => {
				info!(file_uri = file.document_identifier.uri.as_str(), "Background parsing");

				// If the file has been opened in the IDE during the time taken to start this background parse, then
				// ignore it. The IDE is now the source of truth for this file.
				if file.is_open_in_ide() {
					continue;
				}

				let contents = file_system.file_contents(file.document_identifier.uri.clone()).await;

				if let None = contents {
					error!(file_uri = file.document_identifier.uri.as_str(), "Failed to retrieve file contents.");

					continue;
				}

				info!(file_uri = file.document_identifier.uri.as_str(), "Got contents: {}", contents.unwrap());

				let compiled_unit: CompiledUnit = (); // Parse contents.

				// If the file has been opened in the IDE during the fetching and parsing of its contents, then
				// throw it all away. The IDE is now the source of truth for this file. Otherwise, update its
				// compiled unit.
				if !file.is_open_in_ide() {
					file.set_compiled_unit(compiled_unit, None);
				}
			},
			Err(_) => break
		};
	}
}
