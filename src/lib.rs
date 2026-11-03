//! # ghwatch
//!
//! Vigilancia de vulnerabilidades en los repositorios GitHub de una cuenta.
//!
//! Esta biblioteca expone la lógica reutilizable del CLI `ghwatch`. Contiene el
//! módulo [`auth`], encargado de resolver la credencial de GitHub sin exponer el
//! token, y el módulo [`github`], con el cliente HTTP que respeta los límites de
//! la API.

pub mod auth;
pub mod github;
