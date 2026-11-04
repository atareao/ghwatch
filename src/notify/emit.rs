//! Escritura del informe y notificación best-effort a Valet.
//!
//! Este módulo **coordina** dos piezas: el render y la escritura en disco
//! ([`crate::report`]) y el cliente de Valet ([`ValetClient`]). La dependencia es
//! unidireccional —`notify → report`—: el módulo de informes no conoce la red.
//!
//! La regla de oro es que **el informe siempre se escribe primero**: la
//! notificación es best-effort y un fallo suyo no puede dejar el informe sin
//! escribir ni alterar el resultado del escaneo.

use std::io;
use std::path::Path;

use crate::report::{ReportDocument, ReportError, ReportPaths, write_reports};

use super::{ValetClient, build_notification, should_notify};

/// Resultado de la notificación, sin propagar el error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyOutcome {
    /// Enviada; lleva el identificador devuelto por Valet.
    Sent(String),
    /// Omitida por el modo silencioso (sin novedades y sin `--force`).
    Skipped,
    /// No hay notificador configurado.
    NotConfigured,
    /// El envío falló; el informe ya estaba escrito.
    Failed(String),
}

/// Resultado de escribir el informe y (si procede) notificarlo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitOutcome {
    /// Rutas de los informes escritos.
    pub paths: ReportPaths,
    /// Desenlace de la notificación.
    pub notify: NotifyOutcome,
}

/// Escribe el informe y, después, notifica a Valet de forma best-effort.
///
/// El orden es deliberado: primero se persiste el informe —con independencia del
/// resultado de la notificación— y solo después se intenta notificar. Un fallo
/// de Valet se registra como [`NotifyOutcome::Failed`] y **no** se propaga: el
/// escaneo no debe fallar por la notificación.
///
/// La escritura en disco se delega a un hilo bloqueante
/// ([`tokio::task::spawn_blocking`]): no se bloquea el runtime asíncrono con E/S
/// síncrona.
///
/// # Errores
///
/// Devuelve [`ReportError`] solo si falla la escritura del informe. La
/// notificación nunca produce error.
pub async fn emit_and_notify(
    document: &ReportDocument,
    markdown: &Path,
    json: &Path,
    notifier: Option<&ValetClient>,
    force: bool,
) -> Result<EmitOutcome, ReportError> {
    let paths = escribir_informe(document, markdown, json).await?;

    let notify = match notifier {
        None => NotifyOutcome::NotConfigured,
        Some(cliente) => {
            if !should_notify(document.new, document.resolved, force) {
                tracing::debug!(
                    "sin alertas nuevas ni resueltas: no se notifica a Valet (usa --force para forzar)"
                );
                NotifyOutcome::Skipped
            } else {
                enviar(cliente, document, &paths.markdown).await
            }
        }
    };

    Ok(EmitOutcome { paths, notify })
}

/// Escribe el informe en un hilo bloqueante para no bloquear el runtime.
///
/// El documento se clona para poder moverlo al hilo bloqueante: el informe es
/// pequeño y el coste de la copia es despreciable frente a la E/S de disco.
async fn escribir_informe(
    document: &ReportDocument,
    markdown: &Path,
    json: &Path,
) -> Result<ReportPaths, ReportError> {
    let documento = document.clone();
    let markdown = markdown.to_path_buf();
    let json = json.to_path_buf();
    let ruta_error = markdown.clone();

    tokio::task::spawn_blocking(move || write_reports(&documento, &markdown, &json))
        .await
        .map_err(|error| ReportError::Write {
            path: ruta_error,
            source: io::Error::other(error),
        })?
}

/// Compone y envía la notificación, traduciendo el error a [`NotifyOutcome`].
async fn enviar(
    cliente: &ValetClient,
    document: &ReportDocument,
    report_path: &Path,
) -> NotifyOutcome {
    let notificacion = build_notification(document, report_path);
    match cliente.send(&notificacion).await {
        Ok(id) => NotifyOutcome::Sent(id),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "no se pudo notificar a Valet; el informe ya está escrito"
            );
            NotifyOutcome::Failed(error.to_string())
        }
    }
}
