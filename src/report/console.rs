//! Salida de consola del informe.
//!
//! Dibuja una tabla con una fila por alerta, **agrupada por severidad** y, dentro
//! de cada grupo, ordenada por prioridad descendente (la de
//! [`crate::domain::priority`]). El color de la celda de severidad es parte del
//! contrato: se fuerza aunque la salida no sea una terminal interactiva
//! ([`comfy_table::Table::enforce_styling`]), de modo que el color es
//! determinista en tests y en redirecciones a fichero.

use chrono::{DateTime, Utc};
use comfy_table::{Cell, Color, Table};

use crate::domain::alert::{Alert, Severity};
use crate::domain::priority;

/// Orden de presentación de los grupos: de más a menos grave.
const ORDEN: [Severity; 5] = [
    Severity::Critical,
    Severity::High,
    Severity::Medium,
    Severity::Low,
    Severity::Unknown,
];

/// Color asociado a cada severidad.
///
/// Cada severidad conocida tiene un color propio y distinto; las alertas sin
/// severidad usan un gris neutro para no confundirse con ninguna.
#[must_use]
pub fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Critical => Color::Red,
        Severity::High => Color::DarkRed,
        Severity::Medium => Color::Yellow,
        Severity::Low => Color::Green,
        Severity::Unknown => Color::Grey,
    }
}

/// Renderiza la tabla de consola de un conjunto de alertas.
///
/// `now` es la referencia temporal de la priorización, inyectada para que el
/// resultado sea reproducible.
#[must_use]
pub fn render_console(alerts: &[Alert], now: DateTime<Utc>) -> String {
    let mut tabla = Table::new();
    // El color es parte del contrato del informe: se fuerza incluso sin TTY.
    tabla.enforce_styling();
    tabla.set_header(vec!["Severidad", "Fuente", "CVSS", "Paquete", "Resumen"]);

    for severidad in ORDEN {
        let grupo = alerts
            .iter()
            .filter(|alerta| pertenece_a(alerta, severidad));

        for alerta in priority::prioritize(grupo, now) {
            tabla.add_row(vec![
                Cell::new(severidad.as_str()).fg(severity_color(severidad)),
                Cell::new(alerta.source.as_str()),
                Cell::new(cvss_texto(alerta.cvss)),
                Cell::new(alerta.package.as_deref().unwrap_or("—")),
                Cell::new(alerta.summary.as_deref().unwrap_or("—")),
            ]);
        }
    }

    tabla.to_string()
}

/// `true` si la alerta pertenece al grupo de la severidad dada.
///
/// Una alerta sin severidad cuenta como [`Severity::Unknown`].
fn pertenece_a(alerta: &Alert, severidad: Severity) -> bool {
    match (severidad, alerta.severity) {
        (Severity::Unknown, None | Some(Severity::Unknown)) => true,
        (Severity::Unknown, Some(_)) => false,
        (_, Some(suya)) => suya == severidad,
        (_, None) => false,
    }
}

/// Texto de la puntuación CVSS, con un guion cuando no consta.
fn cvss_texto(cvss: Option<f64>) -> String {
    cvss.map_or_else(|| "—".to_string(), |valor| format!("{valor:.1}"))
}
