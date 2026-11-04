//! Orquestación del escaneo.
//!
//! Este módulo une las piezas de las fases anteriores: descubre el inventario
//! ([`crate::github::repos::discover`]), lo filtra ([`RepoSelection`]) y
//! recolecta, por repositorio y con concurrencia acotada, las tres fuentes de
//! alertas —[`crate::github::dependabot`], [`crate::github::secret_scanning`] y
//! [`crate::github::code_scanning`]—.
//!
//! El resultado es un [`ScanSummary`] que separa los repositorios correctos de
//! los fallidos —un fallo individual no aborta el resto del escaneo—, lista los
//! fallidos por nombre y agrega las alertas normalizadas. Al fusionar el informe
//! de Dependabot se actualiza el estado de cada [`Repo`] (`enabled`/`disabled`),
//! que el descubrimiento deja en `unknown`.
//!
//! # Progreso
//!
//! El avance se comunica a través del trait [`ProgressReporter`], inyectable. La
//! **vía de producción** es [`IndicatifReporter`], una barra de `indicatif`; en
//! tests y salidas no interactivas se usa [`HiddenReporter`], que no dibuja
//! nada. Así la orquestación no depende de un TTY ni ensucia la salida de los
//! tests.
//!
//! # Concurrencia
//!
//! La concurrencia la acota el semáforo del propio [`GithubClient`] a través de
//! [`GithubClient::for_each_concurrent`], que además conserva el orden del
//! inventario en el resultado.

use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use tokio::task::JoinError;

use crate::domain::alert::Alert;
use crate::domain::repo::Repo;
use crate::github::GithubError;
use crate::github::client::GithubClient;
use crate::github::code_scanning::{self, CodeScanningReport};
use crate::github::dependabot;
use crate::github::repos::{self, RepoSelection};
use crate::github::secret_scanning::{self, SecretScanningReport};
use crate::osv::OsvClient;

/// Recolección de las tres fuentes de alertas de un repositorio.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoScan {
    /// Metadato del repositorio, con el estado de Dependabot ya fusionado.
    ///
    /// El estado de Dependabot vive **solo** aquí (`repo.dependabot`): las
    /// alertas se guardan aparte, en [`RepoScan::dependabot_alerts`], de modo
    /// que no hay dos copias del estado que puedan desincronizarse.
    pub repo: Repo,
    /// Alertas normalizadas de Dependabot; vacío si la capacidad está
    /// deshabilitada.
    pub dependabot_alerts: Vec<Alert>,
    /// Informe de secret scanning: si la fuente era observable y sus alertas.
    pub secrets: SecretScanningReport,
    /// Informe de code scanning: si hay análisis configurado y sus alertas.
    pub code: CodeScanningReport,
}

impl RepoScan {
    /// Número total de alertas normalizadas del repositorio.
    #[must_use]
    pub fn alert_count(&self) -> usize {
        self.dependabot_alerts.len() + self.secrets.alerts.len() + self.code.alerts.len()
    }

    /// Itera las alertas normalizadas de las tres fuentes, en orden.
    pub fn alerts(&self) -> impl Iterator<Item = &Alert> {
        self.dependabot_alerts
            .iter()
            .chain(&self.secrets.alerts)
            .chain(&self.code.alerts)
    }

    /// Itera mutablemente las alertas normalizadas de las tres fuentes, en orden.
    ///
    /// Lo usa el enriquecimiento para completar las alertas **in situ** tras la
    /// recolección.
    pub fn alerts_mut(&mut self) -> impl Iterator<Item = &mut Alert> {
        self.dependabot_alerts
            .iter_mut()
            .chain(self.secrets.alerts.iter_mut())
            .chain(self.code.alerts.iter_mut())
    }
}

/// Salida de la tarea de un repositorio: su nombre y el resultado de la
/// recolección.
///
/// El nombre viaja **con** el resultado —la clausura es dueña del [`Repo`]—,
/// de modo que la atribución de un fallo no depende de la posición y no puede
/// degradar en silencio a un nombre vacío o al de otro repositorio.
struct RepoOutcome {
    /// Nombre completo del repositorio (`owner/name`).
    nombre: String,
    /// Recolección del repositorio o el error que la impidió.
    resultado: Result<RepoScan, GithubError>,
}

/// Guarda que avanza el progreso al descolgarse.
///
/// El `Drop` corre también durante el desenrollado de un pánico, así que la
/// barra avanza aunque la tarea de un repositorio falle en el runtime.
struct ProgresoGuard {
    /// Informe de progreso al que se notifica el avance.
    reporter: Arc<dyn ProgressReporter>,
}

impl Drop for ProgresoGuard {
    fn drop(&mut self) {
        self.reporter.advance(1);
    }
}

/// Fallo de un repositorio durante el escaneo.
#[derive(Debug, Clone)]
pub struct ScanFailure {
    /// Nombre completo del repositorio (`owner/name`).
    pub repo: String,
    /// Error que impidió recolectar sus alertas.
    pub error: GithubError,
}

/// Resumen de un escaneo.
#[derive(Debug, Clone, Default)]
pub struct ScanSummary {
    /// Repositorios recolectados con éxito, en el orden del inventario.
    pub scanned: Vec<RepoScan>,
    /// Repositorios que fallaron, en el orden del inventario.
    pub failed: Vec<ScanFailure>,
}

impl ScanSummary {
    /// Número de repositorios correctos.
    #[must_use]
    pub fn succeeded_count(&self) -> usize {
        self.scanned.len()
    }

    /// Número de repositorios fallidos.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    /// Nombres de los repositorios fallidos, en orden.
    #[must_use]
    pub fn failed_names(&self) -> Vec<&str> {
        self.failed
            .iter()
            .map(|fallo| fallo.repo.as_str())
            .collect()
    }

    /// Número total de alertas normalizadas recolectadas.
    #[must_use]
    pub fn alert_count(&self) -> usize {
        self.scanned.iter().map(RepoScan::alert_count).sum()
    }

    /// Itera todas las alertas normalizadas recolectadas, en orden.
    pub fn alerts(&self) -> impl Iterator<Item = &Alert> {
        self.scanned.iter().flat_map(RepoScan::alerts)
    }
}

/// Comunicación del progreso del escaneo.
///
/// El escaneo solo depende de este trait, de modo que la barra de progreso es
/// inyectable y sustituible por un doble en los tests.
pub trait ProgressReporter: Send + Sync {
    /// Arranca el informe con el número total de repositorios.
    fn start(&self, total: u64);
    /// Registra `delta` repositorios procesados.
    fn advance(&self, delta: u64);
    /// Cierra el informe al terminar el escaneo.
    fn finish(&self);
}

/// Vía de **producción** del progreso: una barra de `indicatif`.
///
/// En una terminal dibuja la barra; cuando la salida no es interactiva,
/// `indicatif` no escribe nada, así que también sirve de forma segura en
/// entornos sin TTY.
pub struct IndicatifReporter {
    /// Barra subyacente.
    bar: ProgressBar,
}

impl IndicatifReporter {
    /// Crea la barra de progreso.
    #[must_use]
    pub fn new() -> Self {
        let bar = ProgressBar::new(0);
        // Si la plantilla no fuese válida se conserva el estilo por defecto en
        // lugar de abortar: el progreso es accesorio, nunca crítico.
        let estilo = ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} repos ({eta})",
        )
        .map_or_else(
            |_| ProgressStyle::default_bar(),
            |estilo| estilo.progress_chars("=> "),
        );
        bar.set_style(estilo);
        Self { bar }
    }
}

impl Default for IndicatifReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressReporter for IndicatifReporter {
    fn start(&self, total: u64) {
        self.bar.set_length(total);
        self.bar.set_position(0);
    }

    fn advance(&self, delta: u64) {
        self.bar.inc(delta);
    }

    fn finish(&self) {
        self.bar.finish_with_message("escaneo completado");
    }
}

/// Informe de progreso que no dibuja nada.
///
/// Doble para tests y para salidas no interactivas (sin TTY).
#[derive(Debug, Default, Clone, Copy)]
pub struct HiddenReporter;

impl ProgressReporter for HiddenReporter {
    fn start(&self, _total: u64) {}
    fn advance(&self, _delta: u64) {}
    fn finish(&self) {}
}

/// Ejecuta un escaneo completo sobre la cuenta autenticada.
///
/// Descubre el inventario, aplica `selection`, recolecta las tres fuentes por
/// repositorio con la concurrencia acotada del cliente y devuelve el
/// [`ScanSummary`]. No realiza enriquecimiento: para completar las alertas con
/// OSV, usa [`scan_with_enrichment`].
///
/// El cliente se recibe en un [`Arc`] porque
/// [`GithubClient::for_each_concurrent`] ejecuta cada repositorio en una tarea
/// propia (con vida `'static`): compartirlo entre tareas exige propiedad
/// compartida.
///
/// # Errores
///
/// Devuelve el [`GithubError`] del descubrimiento si no se puede enumerar el
/// inventario. Los fallos **por repositorio** no abortan el escaneo: se recogen
/// en [`ScanSummary::failed`].
pub async fn scan(
    client: Arc<GithubClient>,
    selection: RepoSelection,
    reporter: Arc<dyn ProgressReporter>,
) -> Result<ScanSummary, GithubError> {
    scan_inner(client, selection, reporter, None).await
}

/// Ejecuta un escaneo completo y enriquece sus alertas con OSV.
///
/// Es la variante de [`scan`] con enriquecimiento **best-effort**: tras
/// recolectar, completa las alertas con el catálogo de OSV. Si OSV falla o
/// expira, el escaneo **concluye igualmente**: se registra un aviso y las
/// alertas se reportan sin enriquecer. El enriquecimiento nunca aborta el
/// escaneo.
pub async fn scan_with_enrichment(
    client: Arc<GithubClient>,
    selection: RepoSelection,
    reporter: Arc<dyn ProgressReporter>,
    enricher: Arc<OsvClient>,
) -> Result<ScanSummary, GithubError> {
    scan_inner(client, selection, reporter, Some(enricher)).await
}

/// Núcleo del escaneo, con enriquecimiento opcional.
async fn scan_inner(
    client: Arc<GithubClient>,
    selection: RepoSelection,
    reporter: Arc<dyn ProgressReporter>,
    enricher: Option<Arc<OsvClient>>,
) -> Result<ScanSummary, GithubError> {
    let inventario = repos::discover(&client).await?;
    let seleccionados = repos::select(inventario, selection);

    reporter.start(seleccionados.len() as u64);

    let client_tarea = Arc::clone(&client);
    let reporter_tarea = Arc::clone(&reporter);
    let resultados = client
        .for_each_concurrent(seleccionados, move |repo| {
            let client = Arc::clone(&client_tarea);
            let reporter = Arc::clone(&reporter_tarea);
            async move {
                // El progreso avanza pase lo que pase: el guarda cubre también
                // la salida por pánico, que no alcanzaría una llamada posterior.
                let _guarda = ProgresoGuard { reporter };
                // El nombre se captura antes de ceder el `Repo`: viaja con la
                // salida en lugar de re-derivarse por posición.
                let nombre = repo.name_with_owner.clone();
                let resultado = collect_repo(&client, repo).await;
                RepoOutcome { nombre, resultado }
            }
        })
        .await;

    let mut resumen = aggregate(resultados);
    if let Some(enricher) = enricher {
        enriquecer(&enricher, &mut resumen).await;
    }
    reporter.finish();
    Ok(resumen)
}

/// Completa las alertas del resumen con OSV, best-effort.
///
/// Un fallo de OSV no aborta el escaneo: se registra un aviso y las alertas
/// quedan sin enriquecer (la fusión solo se aplica si la respuesta llega
/// completa).
async fn enriquecer(enricher: &OsvClient, resumen: &mut ScanSummary) {
    let alertas: Vec<&mut Alert> = resumen
        .scanned
        .iter_mut()
        .flat_map(RepoScan::alerts_mut)
        .collect();

    match enricher.enrich(alertas).await {
        Ok(resultado) => tracing::debug!(
            consultadas = resultado.queried,
            enriquecidas = resultado.enriched,
            omitidas = resultado.skipped,
            "enriquecimiento OSV completado"
        ),
        Err(error) => tracing::warn!(
            error = %error,
            "OSV no disponible: el escaneo continúa con las alertas sin enriquecer"
        ),
    }
}

/// Agrega las salidas de las tareas en un [`ScanSummary`], conservando el orden
/// del inventario.
///
/// Un [`JoinError`] (pánico o cancelación de la tarea) no tiene identidad de
/// repositorio: se etiqueta por su posición —nunca con un nombre vacío— y se
/// registra su causa para no perder el diagnóstico.
fn aggregate(resultados: Vec<Result<RepoOutcome, JoinError>>) -> ScanSummary {
    let mut resumen = ScanSummary::default();

    for (indice, resultado) in resultados.into_iter().enumerate() {
        match resultado {
            Ok(salida) => match salida.resultado {
                Ok(repositorio) => resumen.scanned.push(repositorio),
                Err(error) => resumen.failed.push(ScanFailure {
                    repo: salida.nombre,
                    error,
                }),
            },
            Err(join) => {
                tracing::error!(
                    repositorio = indice,
                    error = %join,
                    "la tarea del repositorio falló en el runtime"
                );
                resumen.failed.push(ScanFailure {
                    repo: format!("<repositorio #{indice}>"),
                    error: GithubError::TaskFailed(join.to_string()),
                });
            }
        }
    }

    resumen
}

/// Recolecta las tres fuentes de un repositorio y fusiona su estado de
/// Dependabot en el metadato.
///
/// Recibe el [`Repo`] **por valor**: la tarea ya es dueña de él, así que no hay
/// que clonarlo.
///
/// Se detiene en el primer error: si una fuente falla, el repositorio se
/// considera fallido y no se consultan las restantes.
async fn collect_repo(client: &GithubClient, repo: Repo) -> Result<RepoScan, GithubError> {
    let nombre = repo.name_with_owner.as_str();
    let dependabot = dependabot::collect(client, nombre).await?;
    let secrets = secret_scanning::collect(client, nombre).await?;
    let code = code_scanning::collect(client, nombre).await?;

    let mut metadato = repo;
    metadato.dependabot = dependabot.state;

    Ok(RepoScan {
        repo: metadato,
        dependabot_alerts: dependabot.alerts,
        secrets,
        code,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use tokio::task::JoinSet;

    use super::*;
    use crate::domain::alert::{Alert, AlertSource};
    use crate::domain::repo::DependabotState;

    /// Doble de progreso que solo contabiliza los avances.
    #[derive(Default)]
    struct ContadorAvances {
        avances: AtomicU64,
    }

    impl ProgressReporter for ContadorAvances {
        fn start(&self, _total: u64) {}
        fn advance(&self, delta: u64) {
            self.avances.fetch_add(delta, Ordering::SeqCst);
        }
        fn finish(&self) {}
    }

    /// Alerta mínima de la fuente indicada.
    fn alerta(source: AlertSource, external_id: &str) -> Alert {
        Alert {
            source,
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

    /// Recolección de un repositorio con `extras` alertas de secreto además de
    /// una alerta de Dependabot.
    fn repo_scan(nombre: &str, dependabot: DependabotState, extras: usize) -> RepoScan {
        RepoScan {
            repo: Repo {
                name_with_owner: nombre.to_string(),
                private: false,
                archived: false,
                fork: false,
                language: None,
                pushed_at: None,
                dependabot,
            },
            dependabot_alerts: vec![alerta(AlertSource::Dependabot, "1")],
            secrets: SecretScanningReport {
                observable: true,
                alerts: (0..extras)
                    .map(|indice| alerta(AlertSource::Secret, &indice.to_string()))
                    .collect(),
            },
            code: CodeScanningReport {
                configured: true,
                alerts: Vec::new(),
            },
        }
    }

    /// El resumen cuenta correctos, fallidos y alertas, y lista los fallidos.
    #[test]
    fn el_resumen_cuenta_correctos_fallidos_y_alertas() {
        let resumen = ScanSummary {
            scanned: vec![repo_scan("atareao/uno", DependabotState::Enabled, 2)],
            failed: vec![ScanFailure {
                repo: "atareao/dos".to_string(),
                error: GithubError::NotFound,
            }],
        };

        assert_eq!(resumen.succeeded_count(), 1);
        assert_eq!(resumen.failed_count(), 1);
        assert_eq!(resumen.failed_names(), vec!["atareao/dos"]);
        assert_eq!(resumen.alert_count(), 3);
        assert_eq!(resumen.alerts().count(), 3);
    }

    /// El fallo de una tarea (pánico) no deja el nombre vacío: se etiqueta por
    /// su posición y se conserva la causa del `JoinError` en un error tipado
    /// que no se confunde con un fallo de transporte.
    #[tokio::test]
    async fn el_panico_de_una_tarea_no_deja_nombre_vacio_y_conserva_la_causa() {
        let correcto = RepoOutcome {
            nombre: "atareao/uno".to_string(),
            resultado: Err(GithubError::NotFound),
        };

        let mut conjunto: JoinSet<()> = JoinSet::new();
        conjunto.spawn(async { panic!("pánico deliberado de prueba") });
        let panico = conjunto
            .join_next()
            .await
            .expect("debe completarse una tarea")
            .expect_err("la tarea debe fallar en el runtime");

        let resumen = aggregate(vec![Ok(correcto), Err(panico)]);

        assert_eq!(resumen.failed_count(), 2);
        assert!(
            resumen.failed.iter().all(|fallo| !fallo.repo.is_empty()),
            "ningún fallo debe salir con nombre vacío"
        );
        assert_eq!(resumen.failed[0].repo, "atareao/uno");
        assert_eq!(resumen.failed[1].repo, "<repositorio #1>");
        match &resumen.failed[1].error {
            GithubError::TaskFailed(causa) => {
                assert!(!causa.is_empty(), "se conserva la causa del JoinError");
            }
            otro => panic!("se esperaba TaskFailed, no: {otro:?}"),
        }
    }

    /// El progreso avanza también cuando la tarea de un repositorio entra en
    /// pánico: el guarda avanza al descolgarse durante el desenrollado.
    #[tokio::test]
    async fn el_progreso_avanza_aunque_la_tarea_entre_en_panico() {
        let reporter = Arc::new(ContadorAvances::default());
        let reporter_tarea: Arc<dyn ProgressReporter> = reporter.clone();

        let tarea = tokio::spawn(async move {
            let _guarda = ProgresoGuard {
                reporter: reporter_tarea,
            };
            panic!("pánico deliberado de prueba");
        });

        assert!(tarea.await.is_err(), "la tarea debe fallar en el runtime");
        assert_eq!(
            reporter.avances.load(Ordering::SeqCst),
            1,
            "el progreso avanza también en la ruta de pánico"
        );
    }
}
