//! Library errors. Messages carry no file paths, so they are safe to show in the UI.

/// Everything the library layer can fail with.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    /// The database was written by a newer Aria and is left untouched.
    #[error("the library database comes from a newer version of Aria")]
    NewerSchema {
        /// Schema version found in the file.
        found: u32,
        /// Highest schema version this build understands.
        supported: u32,
    },
    /// SQLite reported a failure.
    #[error("library database error")]
    Database(#[source] rusqlite::Error),
    /// A file system operation failed.
    #[error("library file system error")]
    Io(#[source] std::io::Error),
    /// A file's tags or properties could not be read.
    #[error("the file's tags could not be read")]
    Tags(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The folder lies inside a folder that is already in the library.
    #[error("the folder is inside a folder that is already in the library")]
    FolderInsideExisting,
    /// The folder is already in the library.
    #[error("the folder is already in the library")]
    FolderAlreadyAdded,
    /// A playlist name is empty or longer than 100 characters after trimming.
    #[error("the playlist name must be 1 to 100 characters")]
    InvalidPlaylistName,
}

impl From<rusqlite::Error> for LibraryError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}

impl From<std::io::Error> for LibraryError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
