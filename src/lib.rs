//! # ghwatch
//!
//! Vigilancia de vulnerabilidades en los repositorios GitHub de una cuenta.
//!
//! Esta biblioteca expone la lógica reutilizable del CLI `ghwatch`. Contiene el
//! módulo [`auth`], encargado de resolver la credencial de GitHub sin exponer el
//! token; el módulo [`github`], con el cliente HTTP que respeta los límites de
//! la API y la recolección de alertas; el módulo [`domain`], con los tipos
//! puros (modelo único de alerta e inventario de repositorios) de los que
//! dependen los demás; el módulo [`scan`], que orquesta el descubrimiento y la
//! recolección sobre el inventario completo; el módulo [`osv`], que enriquece
//! las alertas con el catálogo público de OSV.dev; el módulo [`store`], que
//! persiste el histórico en SQLite y calcula la deriva entre ejecuciones; el
//! módulo [`report`], que presenta el estado actual y la evolución en consola,
//! Markdown y JSON; y el módulo [`notify`], que entrega el resumen a Valet de
//! forma best-effort. El módulo [`config`] resuelve la configuración de
//! ejecución y [`cli`] expone los subcomandos y su orquestación.

pub mod auth;
pub mod cli;
pub mod config;
pub mod domain;
pub mod github;
pub mod notify;
pub mod osv;
pub mod report;
pub mod scan;
pub mod store;
