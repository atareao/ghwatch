//! Deriva del ciclo de vida de las alertas.
//!
//! El modelo único de [`Alert`](super::Alert) no tiene noción de estado: el
//! ciclo de vida lo aporta la persistencia. Este módulo define las piezas
//! **puras** de ese ciclo: el [`AlertState`] de una alerta almacenada, la
//! categoría de [`Drift`] y la función que la clasifica ([`Drift::classify`]).
//!
//! La clasificación compara el estado previo de una alerta con su presencia y
//! severidad en la ejecución actual. El conjunto de categorías es **cerrado**:
//! `new`, `still_open`, `resolved`, `reopened` y `severity_up`.

use serde::{Deserialize, Serialize};

use super::alert::Severity;

/// Estado de ciclo de vida de una alerta almacenada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    /// La alerta seguía presente en el último escaneo que la vio.
    Open,
    /// La alerta desapareció de un escaneo posterior.
    Resolved,
}

impl AlertState {
    /// Representación textual estable, en `snake_case`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
        }
    }

    /// Interpreta la representación textual; `None` si no se reconoce.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "resolved" => Some(Self::Resolved),
            _ => None,
        }
    }
}

/// Categoría de deriva de una alerta entre dos ejecuciones.
///
/// Conjunto **cerrado**: toda alerta en juego se clasifica en exactamente una de
/// estas categorías. La única excepción es la alerta que ya estaba resuelta y
/// sigue ausente: no representa ningún cambio y no genera evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Drift {
    /// La alerta aparece por primera vez.
    New,
    /// La alerta ya estaba abierta y sigue presente.
    StillOpen,
    /// La alerta estaba abierta y ha desaparecido.
    Resolved,
    /// La alerta estaba resuelta y ha reaparecido.
    Reopened,
    /// La alerta sigue abierta y su severidad ha subido.
    SeverityUp,
}

impl Drift {
    /// Todas las categorías del conjunto cerrado, en orden.
    pub const ALL: [Self; 5] = [
        Self::New,
        Self::StillOpen,
        Self::Resolved,
        Self::Reopened,
        Self::SeverityUp,
    ];

    /// Representación textual estable, en `snake_case`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::StillOpen => "still_open",
            Self::Resolved => "resolved",
            Self::Reopened => "reopened",
            Self::SeverityUp => "severity_up",
        }
    }

    /// Interpreta la representación textual; `None` si no se reconoce.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|categoria| categoria.as_str() == value)
    }

    /// Clasifica la deriva de una alerta.
    ///
    /// `previous` es el estado almacenado de la alerta (si ya existía). `current`
    /// es la severidad observada **ahora**: `None` significa que la alerta no ha
    /// aparecido; si aparece pero la fuente no aporta severidad, se pasa
    /// [`Severity::Unknown`] (nunca `None`), porque la presencia no depende de
    /// que la severidad conste.
    ///
    /// Devuelve `None` cuando no hay cambio que registrar: la alerta ya estaba
    /// resuelta y sigue ausente.
    ///
    /// Una reapertura se clasifica como [`Drift::Reopened`], nunca como
    /// [`Drift::New`]: la alerta ya existía y tiene un `first_seen_run` previo.
    #[must_use]
    pub fn classify(previous: Option<PriorAlert>, current: Option<Severity>) -> Option<Self> {
        match (previous, current) {
            (None, Some(_)) => Some(Self::New),
            (None, None) => None,
            (Some(previo), Some(severidad)) => match previo.state {
                AlertState::Resolved => Some(Self::Reopened),
                AlertState::Open => {
                    if ha_escalado(previo.severity, severidad) {
                        Some(Self::SeverityUp)
                    } else {
                        Some(Self::StillOpen)
                    }
                }
            },
            (Some(previo), None) => match previo.state {
                AlertState::Open => Some(Self::Resolved),
                AlertState::Resolved => None,
            },
        }
    }
}

/// `true` si la severidad actual es más alta que la previa.
fn ha_escalado(anterior: Option<Severity>, actual: Severity) -> bool {
    actual.rank() > anterior.map_or(0, Severity::rank)
}

/// Estado previo de una alerta, tal como quedó almacenado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorAlert {
    /// Estado del ciclo de vida.
    pub state: AlertState,
    /// Severidad con la que se almacenó, si consta.
    pub severity: Option<Severity>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El orden de severidad crece de `Unknown` a `Critical`.
    #[test]
    fn severity_rank_ordena_de_menor_a_mayor() {
        assert!(Severity::Unknown.rank() < Severity::Low.rank());
        assert!(Severity::Low.rank() < Severity::Medium.rank());
        assert!(Severity::Medium.rank() < Severity::High.rank());
        assert!(Severity::High.rank() < Severity::Critical.rank());
    }

    /// Las representaciones textuales coinciden con la serialización `serde`.
    #[test]
    fn las_representaciones_textuales_coinciden_con_serde() {
        for categoria in Drift::ALL {
            assert_eq!(
                serde_json::to_string(&categoria).expect("serializable"),
                format!("\"{}\"", categoria.as_str())
            );
        }
    }

    /// El origen de la alerta se representa y se lee de vuelta.
    #[test]
    fn alert_source_ida_y_vuelta() {
        for (origen, texto) in [
            (super::super::alert::AlertSource::Dependabot, "dependabot"),
            (super::super::alert::AlertSource::Secret, "secret"),
            (super::super::alert::AlertSource::Code, "code"),
        ] {
            assert_eq!(origen.as_str(), texto);
            assert_eq!(super::super::alert::AlertSource::parse(texto), Some(origen));
        }
        assert_eq!(super::super::alert::AlertSource::parse("otro"), None);
    }
}
