//! Priorización determinista de alertas.
//!
//! Ordena las alertas por una puntuación que combina cinco factores: severidad,
//! CVSS, existencia de versión corregida, `scope` (`runtime` por encima de
//! `development`) y **antigüedad**. La función es **pura**: no toca red ni disco.
//!
//! # Antigüedad
//!
//! La antigüedad se mide contra un instante de referencia **inyectado**
//! (`now`), de modo que el resultado es reproducible sin depender del reloj real
//! y los tests no son sensibles al tiempo. El instante de creación de cada alerta
//! lo aporta [`Alert::created_at`], poblado desde GitHub donde existe; una alerta
//! sin fecha cuenta como la menos antigua (contribución cero).
//!
//! # Jerarquía y desempate
//!
//! Los pesos son **estrictamente jerárquicos**: cada factor pesa más que la suma
//! de todos los inferiores, así que un factor solo decide cuando todos los
//! superiores empatan. La antigüedad, en particular, está acotada por debajo del
//! escalón del `scope` y crece de forma estrictamente monótona: a igualdad del
//! resto, la más antigua puntúa más alto.
//!
//! El orden es **total y estable**: ante puntuaciones iguales se desempata por
//! **todos** los campos del modelo que no intervienen en la puntuación (origen,
//! identificador, paquete, ecosistema, identificadores, fecha, severidad, CVSS,
//! versión corregida, ámbito, resumen, manifiesto, rango vulnerable y tipo de
//! secreto), nunca por el orden de entrada. Dos alertas con la misma puntuación
//! y los mismos campos quedan en un orden igual, que el ordenamiento estable
//! preserva.

use std::cmp::Ordering;

use chrono::{DateTime, Utc};

use super::alert::{Alert, Severity};

/// Peso de cada escalón de severidad (`Unknown` = 0 … `Critical` = 4).
const PESO_SEVERIDAD: f64 = 100_000.0;
/// Peso del CVSS normalizado a `[0, 1]`.
const PESO_CVSS: f64 = 10_000.0;
/// Peso de disponer de versión corregida.
const PESO_FIX: f64 = 1_000.0;
/// Peso de que el ámbito sea `runtime`.
const PESO_SCOPE: f64 = 100.0;
/// Peso máximo de la antigüedad normalizada (estrictamente menor que `PESO_SCOPE`).
const PESO_ANTIGUEDAD: f64 = 90.0;

/// Puntuación de prioridad de una alerta. Cuanto más alta, más prioritaria.
///
/// `now` es la referencia temporal para medir la antigüedad.
#[must_use]
pub fn score(alert: &Alert, now: DateTime<Utc>) -> f64 {
    let severidad = f64::from(alert.severity.map_or(0, Severity::rank));
    let cvss = alert.cvss.unwrap_or(0.0).clamp(0.0, 10.0) / 10.0;
    let fix = if alert.first_patched.is_some() {
        1.0
    } else {
        0.0
    };
    let scope = if es_runtime(alert.scope.as_deref()) {
        1.0
    } else {
        0.0
    };
    let antiguedad = antiguedad_normalizada(alert.created_at, now);

    PESO_SEVERIDAD * severidad
        + PESO_CVSS * cvss
        + PESO_FIX * fix
        + PESO_SCOPE * scope
        + PESO_ANTIGUEDAD * antiguedad
}

/// Ordena las alertas por prioridad descendente, con desempate total.
///
/// Acepta cualquier iterador de referencias a alertas (por ejemplo `&[Alert]` o
/// el resultado de filtrar `alerts.iter()`), de modo que quien ya trabaja con
/// referencias —como la tabla de consola— no necesita clonar. Devuelve
/// referencias a las alertas en el orden resultante, que es idéntico para los
/// mismos datos con independencia del orden de entrada.
#[must_use]
pub fn prioritize<'a, I>(alerts: I, now: DateTime<Utc>) -> Vec<&'a Alert>
where
    I: IntoIterator<Item = &'a Alert>,
{
    let mut ordenadas: Vec<&Alert> = alerts.into_iter().collect();
    ordenadas.sort_by(|a, b| comparar(a, b, now));
    ordenadas
}

/// Comparador total: puntuación descendente y, a igualdad, campos deterministas.
///
/// El desempate recorre **todos** los campos del modelo que no entran en la
/// puntuación, de modo que el orden es total: dos alertas distintas nunca quedan
/// empatadas salvo que compartan todos sus campos.
fn comparar(a: &Alert, b: &Alert, now: DateTime<Utc>) -> Ordering {
    score(b, now)
        .total_cmp(&score(a, now))
        .then_with(|| a.source.as_str().cmp(b.source.as_str()))
        .then_with(|| a.external_id.cmp(&b.external_id))
        .then_with(|| a.package.cmp(&b.package))
        .then_with(|| a.ecosystem.cmp(&b.ecosystem))
        .then_with(|| a.ghsa_id.cmp(&b.ghsa_id))
        .then_with(|| a.cve_id.cmp(&b.cve_id))
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| {
            a.severity
                .map(Severity::as_str)
                .cmp(&b.severity.map(Severity::as_str))
        })
        .then_with(|| comparar_cvss(a.cvss, b.cvss))
        .then_with(|| a.first_patched.cmp(&b.first_patched))
        .then_with(|| a.scope.cmp(&b.scope))
        .then_with(|| a.summary.cmp(&b.summary))
        .then_with(|| a.manifest_path.cmp(&b.manifest_path))
        .then_with(|| a.vulnerable_range.cmp(&b.vulnerable_range))
        .then_with(|| a.secret_type.cmp(&b.secret_type))
}

/// Orden total sobre el `cvss` opcional (`NaN` no llega aquí: se descarta).
fn comparar_cvss(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// `true` si el ámbito de la dependencia es de ejecución.
fn es_runtime(scope: Option<&str>) -> bool {
    scope.is_some_and(|valor| valor.eq_ignore_ascii_case("runtime"))
}

/// Antigüedad normalizada a `[0, 1)`, estrictamente creciente con los días.
///
/// Una alerta sin fecha conocida contribuye cero. La asíntota en 1 mantiene la
/// contribución por debajo del escalón del `scope`.
fn antiguedad_normalizada(created_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> f64 {
    let Some(created_at) = created_at else {
        return 0.0;
    };
    let dias = (now - created_at).num_seconds().max(0) as f64 / 86_400.0;
    dias / (dias + 365.0)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::domain::alert::AlertSource;

    /// Alerta mínima con el identificador indicado.
    fn alerta(external_id: &str) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: external_id.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: None,
            cvss: None,
            summary: None,
            package: None,
            ecosystem: None,
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// Un instante determinista.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// La jerarquía de pesos es estricta: cada factor supera la suma de los
    /// inferiores. Es una propiedad de los propios pesos, así que se comprueba
    /// en tiempo de compilación.
    #[test]
    fn la_jerarquia_de_pesos_es_estricta() {
        const {
            assert!(PESO_SEVERIDAD > PESO_CVSS + PESO_FIX + PESO_SCOPE + PESO_ANTIGUEDAD);
            assert!(PESO_CVSS > PESO_FIX + PESO_SCOPE + PESO_ANTIGUEDAD);
            assert!(PESO_FIX > PESO_SCOPE + PESO_ANTIGUEDAD);
            assert!(PESO_SCOPE > PESO_ANTIGUEDAD);
        }
    }

    /// La antigüedad es estrictamente creciente y acotada.
    #[test]
    fn la_antiguedad_es_monotona_y_acotada() {
        let now = instante(2_000_000_000);
        let un_dia = antiguedad_normalizada(Some(instante(2_000_000_000 - 86_400)), now);
        let un_ano = antiguedad_normalizada(Some(instante(2_000_000_000 - 365 * 86_400)), now);

        assert!(un_dia < un_ano, "más días, más antigüedad");
        assert!(un_ano < 1.0, "la antigüedad está acotada por debajo de 1");
        assert_eq!(antiguedad_normalizada(None, now), 0.0);
    }

    /// Una fecha futura no genera antigüedad negativa.
    #[test]
    fn una_fecha_futura_no_resta() {
        let now = instante(2_000_000_000);
        assert_eq!(
            antiguedad_normalizada(Some(instante(2_000_000_000 + 86_400)), now),
            0.0
        );
    }

    /// `prioritize` ordena por puntuación descendente.
    #[test]
    fn ordena_por_puntuacion_descendente() {
        let now = instante(2_000_000_000);
        let mut critica = alerta("critica");
        critica.severity = Some(Severity::Critical);
        let mut baja = alerta("baja");
        baja.severity = Some(Severity::Low);

        let alertas = vec![baja, critica];
        let orden = prioritize(&alertas, now);
        assert_eq!(orden[0].external_id, "critica");
        assert_eq!(orden[1].external_id, "baja");
    }
}
