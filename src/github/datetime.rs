//! Interpretación tolerante de las marcas de tiempo de la API de GitHub.
//!
//! GitHub publica los instantes como texto ISO-8601 (`RFC 3339`). Una fecha
//! **ausente** o **ilegible** se traduce a `None` en lugar de fallar: el
//! deserializador de `chrono` rechazaría la respuesta completa y se perderían
//! todas las alertas de un repositorio por un solo campo accesorio. Los
//! colectores guardan el valor crudo y lo normalizan aquí.

use chrono::{DateTime, Utc};

/// Interpreta una fecha ISO-8601 (`RFC 3339`) en UTC.
///
/// Devuelve `None` si el valor está ausente o no es una fecha `RFC 3339`
/// legible. Nunca falla: un dato accesorio no debe abortar la recolección.
#[must_use]
pub(crate) fn parse_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|texto| DateTime::parse_from_rfc3339(texto).ok())
        .map(|fecha| fecha.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una fecha `RFC 3339` se interpreta; lo ausente o ilegible es `None`.
    #[test]
    fn interpreta_una_fecha_y_tolera_lo_ilegible() {
        assert!(parse_rfc3339(Some("2026-11-03T09:00:00Z")).is_some());
        assert!(parse_rfc3339(Some("no es una fecha")).is_none());
        assert!(parse_rfc3339(Some("")).is_none());
        assert!(parse_rfc3339(None).is_none());
    }
}
