//! Tipos de dominio puros de `ghwatch`.
//!
//! Este módulo no hace IO, no conoce la API de GitHub ni la base de datos: son
//! tipos de datos con sus invariantes. La dependencia va siempre en un solo
//! sentido —`github` y (más adelante) `store` y `report` dependen de `domain`,
//! nunca al revés—, de modo que el modelo se puede probar sin red ni disco.
//!
//! Contiene el modelo único de alerta ([`Alert`]) con su origen
//! ([`AlertSource`]) y severidad ([`Severity`]), y el metadato de identidad de
//! un repositorio ([`Repo`]) con su estado de Dependabot ([`DependabotState`]).
//! Añade las piezas puras del ciclo de vida —el estado almacenado
//! ([`AlertState`]) y la categoría de deriva ([`Drift`])—, la instantánea de
//! una ejecución ([`ScanRun`]) y la priorización determinista
//! ([`priority`]).

pub mod alert;
pub mod drift;
pub mod priority;
pub mod repo;
pub mod run;

pub use alert::{Alert, AlertSource, Severity};
pub use drift::{AlertState, Drift, PriorAlert};
pub use repo::{DependabotState, Repo};
pub use run::ScanRun;
