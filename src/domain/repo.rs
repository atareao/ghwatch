//! Repositorios del inventario.
//!
//! El [`Repo`] es el metadato de identidad que se registra tras el
//! descubrimiento: no incluye todavía sus alertas, solo quién es y en qué estado
//! se encuentra su capacidad de Dependabot.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Estado de las alertas de Dependabot en un repositorio.
///
/// Es un conjunto **cerrado** de tres valores: o las alertas están activadas, o
/// GitHub responde que están deshabilitadas, o todavía no se ha podido
/// determinarlo (por ejemplo, antes de consultar el endpoint correspondiente).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependabotState {
    /// El endpoint de alertas responde con normalidad (`200`).
    Enabled,
    /// GitHub responde `403` con el cuerpo de capacidad deshabilitada.
    Disabled,
    /// Aún no se ha consultado, o la consulta no permitió decidirlo.
    #[default]
    Unknown,
}

/// Metadato de identidad de un repositorio del inventario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    /// Nombre completo con propietario (`owner/name`).
    pub name_with_owner: String,
    /// Si el repositorio es privado.
    pub private: bool,
    /// Si el repositorio está archivado.
    pub archived: bool,
    /// Si el repositorio es un fork.
    pub fork: bool,
    /// Lenguaje principal declarado por GitHub, si lo hay.
    pub language: Option<String>,
    /// Último instante en que se empujó contenido, si consta.
    pub pushed_at: Option<DateTime<Utc>>,
    /// Estado conocido de las alertas de Dependabot.
    pub dependabot: DependabotState,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El estado por defecto es `Unknown`: el descubrimiento no consulta alertas.
    #[test]
    fn dependabot_state_por_defecto_es_unknown() {
        assert_eq!(DependabotState::default(), DependabotState::Unknown);
    }

    /// El estado se serializa en `snake_case` como exige el modelo de datos.
    #[test]
    fn dependabot_state_se_serializa_en_snake_case() {
        assert_eq!(
            serde_json::to_string(&DependabotState::Enabled).expect("serializable"),
            "\"enabled\""
        );
        assert_eq!(
            serde_json::to_string(&DependabotState::Disabled).expect("serializable"),
            "\"disabled\""
        );
        assert_eq!(
            serde_json::to_string(&DependabotState::Unknown).expect("serializable"),
            "\"unknown\""
        );
    }
}
