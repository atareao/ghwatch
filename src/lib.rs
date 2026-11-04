//! # ghwatch
//!
//! Vigilancia de vulnerabilidades en los repositorios GitHub de una cuenta.
//!
//! Esta biblioteca expone la lógica reutilizable del CLI `ghwatch`. Contiene el
//! módulo [`auth`], encargado de resolver la credencial de GitHub sin exponer el
//! token; el módulo [`github`], con el cliente HTTP que respeta los límites de
//! la API y la recolección de alertas; el módulo [`domain`], con los tipos
//! puros (modelo único de alerta e inventario de repositorios) de los que
//! dependen los demás; y el módulo [`scan`], que orquesta el descubrimiento y la
//! recolección sobre el inventario completo. Cierra el módulo [`store`], que
//! persiste el histórico en SQLite y calcula la deriva entre ejecuciones.

pub mod auth;
pub mod domain;
pub mod github;
pub mod scan;
pub mod store;
