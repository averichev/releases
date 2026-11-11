//! Use-case orchestration. HTTP status codes and response DTOs do not belong here.
pub mod health;

pub mod applications;
pub mod auth;
pub mod error;
pub mod sessions;
pub mod users;
