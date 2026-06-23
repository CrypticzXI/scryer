//! GraphQL API module boundaries.
//!
//! The monolithic `lib.rs` implementation was split into focused modules to align
//! with the architecture guidance while preserving the same public schema and
//! resolver behavior.

pub mod context;
pub mod mutation;
pub mod query;
pub mod subscription;
pub mod utils;

pub use scryer_interface_media::{mappers, types};

pub use context::{
    ApiContext, ApiSchema, LogBuffer, RestoreContext, RestoreRestartHandle, build_schema,
    build_schema_with_log_buffer, build_schema_with_log_buffer_and_restore, export_schema_sdl,
};
