//! Enriquecimiento de alertas con el catálogo público de OSV.dev.
//!
//! Este módulo es **complementario**: OSV no es una fuente de alertas, sino un
//! catálogo que completa las que GitHub ya aporta. Sus piezas son:
//!
//! - [`ecosystem`]: la tabla cerrada de traducción de ecosistemas.
//! - [`cvss`]: la puntuación base de CVSS v3 a partir de su vector.
//! - [`client`]: el cliente HTTP de `POST /v1/querybatch`, sin credenciales y
//!   por lotes de como máximo 1000 consultas.
//! - [`enrich`]: la fusión **sin sobreescribir** sobre el modelo de alerta.
//!
//! El enriquecimiento es best-effort: un fallo de OSV no aborta el escaneo. La
//! integración en la orquestación vive en
//! [`crate::scan::scan_with_enrichment`].

pub mod client;
pub mod cvss;
pub mod ecosystem;
pub mod enrich;
pub mod model;

pub use client::{DEFAULT_BASE_URL as DEFAULT_OSV_BASE_URL, OsvClient, OsvConfig, OsvError};
pub use ecosystem::map_ecosystem;
pub use enrich::{EnrichmentOutcome, apply_advisory, query_for_alert};
pub use model::{OsvAdvisory, OsvQuery};
