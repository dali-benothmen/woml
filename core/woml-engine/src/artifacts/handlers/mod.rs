//! Private artifact capability-handler facade retaining the original public types.

mod put;
mod read;

pub use put::ManagedArtifactPutHandler;
pub(in crate::artifacts) use read::ArtifactReadOperation;
pub use read::ManagedArtifactReadHandler;
