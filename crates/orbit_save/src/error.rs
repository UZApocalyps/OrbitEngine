//! Save errors.

use thiserror::Error;

use crate::manager::SlotId;
use crate::storage::StorageError;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SaveError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The slot holds no save.
    #[error("slot {0} is empty")]
    NotFound(SlotId),
    /// The bytes are not a valid save (truncated, damaged, wrong file).
    #[error("corrupt save: {0}")]
    Corrupt(String),
    /// A valid save that cannot be used with this build or story.
    #[error("incompatible save: {0}")]
    Incompatible(String),
    #[error("invalid slot {0}")]
    InvalidSlot(SlotId),
    #[error("cannot encode: {0}")]
    Encode(String),
}
