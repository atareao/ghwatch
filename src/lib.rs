//! # ghwatch
//!
//! Vigilancia de vulnerabilidades en los repositorios GitHub de una cuenta.
//!
//! Esta biblioteca expone la lógica reutilizable del CLI `ghwatch`. De momento
//! contiene únicamente el módulo [`auth`], encargado de resolver la credencial
//! de GitHub sin exponer el token.

pub mod auth;
