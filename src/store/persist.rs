//! Persistencia de un escaneo y cálculo de su deriva.
//!
//! [`Store::persist_scan`] es la operación de alto nivel: crea la fila de
//! `scan_runs`, hace *upsert* de repositorios y alertas, calcula la deriva
//! respecto a la ejecución anterior, escribe los `alert_events` y actualiza el
//! ciclo de vida. Todo ocurre dentro de una transacción: o se guarda el escaneo
//! entero, o no se guarda nada.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::domain::alert::{AlertSource, Severity};
use crate::domain::drift::{AlertState, Drift, PriorAlert};
use crate::domain::repo::DependabotState;
use crate::domain::{Alert, Repo};
use crate::scan::ScanSummary;

use super::{SCHEMA_VERSION, Store, StoreError};

/// Clave única de idempotencia de una alerta: `(source, repo, external_id)`.
type Clave = (String, String, String);

/// Fila previa de una alerta: `(id, severity, state)`.
type FilaAlerta = (i64, Option<String>, String);

/// Fuentes que un repositorio permitió observar en esta ejecución.
///
/// Una fuente no observada (capacidad deshabilitada, sin análisis o sin
/// permiso para verla) no puede declarar resueltas sus alertas: su ausencia
/// significa «no se pudo mirar», no «ya no está».
#[derive(Debug, Clone, Copy)]
struct FuentesObservadas {
    /// Las alertas de Dependabot estaban habilitadas.
    dependabot: bool,
    /// La fuente de secret scanning era observable.
    secret: bool,
    /// Había un análisis de code scanning configurado.
    code: bool,
}

/// Resultado de persistir un escaneo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistOutcome {
    /// Identificador de la ejecución creada.
    pub run_id: i64,
    /// Alertas presentes al terminar el escaneo.
    pub alerts_total: usize,
    /// Alertas que aparecen por primera vez.
    pub new: usize,
    /// Alertas que ya estaban abiertas y siguen presentes.
    pub still_open: usize,
    /// Alertas que estaban abiertas y han desaparecido.
    pub resolved: usize,
    /// Alertas que estaban resueltas y han reaparecido.
    pub reopened: usize,
    /// Alertas cuya severidad ha subido.
    pub severity_up: usize,
}

impl PersistOutcome {
    /// Suma un cambio al informe.
    fn registrar(&mut self, cambio: Drift) {
        match cambio {
            Drift::New => self.new += 1,
            Drift::StillOpen => self.still_open += 1,
            Drift::Resolved => self.resolved += 1,
            Drift::Reopened => self.reopened += 1,
            Drift::SeverityUp => self.severity_up += 1,
        }
    }
}

impl Store {
    /// Persiste un escaneo completo y devuelve el informe de deriva.
    ///
    /// Crea la fila de `scan_runs`, hace *upsert* de los repositorios y de las
    /// alertas, calcula la deriva respecto a la ejecución anterior, escribe los
    /// `alert_events` y actualiza el ciclo de vida. La operación es
    /// **idempotente**: repetir un escaneo idéntico no inserta ni resuelve
    /// ninguna alerta y conserva el `first_seen_run`.
    ///
    /// Solo se resuelven alertas de repositorios **escaneados con éxito** y cuya
    /// fuente estaba **observable**: ni un repositorio que falla ni una capacidad
    /// no observable (Dependabot apagado, secret scanning con `404`/`403`, code
    /// scanning sin análisis) provocan falsos `resolved`.
    ///
    /// # Limitación conocida
    ///
    /// Un repositorio que sale del inventario (por ejemplo, un fork nuevo, un
    /// repo archivado o uno que deja de cumplir el filtro) no se escanea, así
    /// que sus alertas abiertas **no** se resuelven: quedan «no observadas».
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla cualquier sentencia; la transacción deja
    /// la base de datos sin cambios parciales.
    pub async fn persist_scan(
        &self,
        scan: &ScanSummary,
        started_at: DateTime<Utc>,
        finished_at: DateTime<Utc>,
    ) -> Result<PersistOutcome, StoreError> {
        let mut tx = self.pool.begin().await?;

        let total_repos = (scan.succeeded_count() + scan.failed_count()) as i64;
        let alerts_total = scan.alert_count();

        let insercion = sqlx::query(
            "INSERT INTO scan_runs \
             (started_at, finished_at, total_repos, ok, failed, alerts_total, schema_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(started_at.to_rfc3339())
        .bind(finished_at.to_rfc3339())
        .bind(total_repos)
        .bind(scan.succeeded_count() as i64)
        .bind(scan.failed_count() as i64)
        .bind(alerts_total as i64)
        .bind(SCHEMA_VERSION)
        .execute(&mut *tx)
        .await?;

        let mut outcome = PersistOutcome {
            run_id: insercion.last_insert_rowid(),
            alerts_total,
            ..PersistOutcome::default()
        };

        let mut observadas: HashMap<String, FuentesObservadas> = HashMap::new();
        let mut presentes: HashSet<Clave> = HashSet::new();

        for repo_scan in &scan.scanned {
            let repo = &repo_scan.repo;
            upsert_repo(&mut tx, repo, finished_at).await?;
            observadas.insert(
                repo.name_with_owner.clone(),
                FuentesObservadas {
                    dependabot: matches!(repo.dependabot, DependabotState::Enabled),
                    secret: repo_scan.secrets.observable,
                    code: repo_scan.code.configured,
                },
            );

            for alert in repo_scan.alerts() {
                presentes.insert(clave(&repo.name_with_owner, alert));
                if let Some(cambio) =
                    persist_alert(&mut tx, &repo.name_with_owner, alert, outcome.run_id).await?
                {
                    outcome.registrar(cambio);
                }
            }
        }

        resolver_desaparecidas(
            &mut tx,
            &observadas,
            &presentes,
            outcome.run_id,
            finished_at,
            &mut outcome,
        )
        .await?;

        tx.commit().await?;
        Ok(outcome)
    }
}

/// Clave única de idempotencia de una alerta dentro de un repositorio.
fn clave(repo: &str, alert: &Alert) -> Clave {
    (
        alert.source.as_str().to_string(),
        repo.to_string(),
        alert.external_id.clone(),
    )
}

/// Severidad observada de una alerta presente en el escaneo.
///
/// Una alerta presente sin severidad aportada cuenta como [`Severity::Unknown`]:
/// la presencia no depende de que la fuente publique la severidad.
fn severidad_observada(alert: &Alert) -> Severity {
    alert.severity.unwrap_or(Severity::Unknown)
}

/// Inserta o actualiza el metadato de un repositorio.
async fn upsert_repo(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    repo: &Repo,
    scanned_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    let alerts_enabled = matches!(repo.dependabot, DependabotState::Enabled);

    sqlx::query(
        "INSERT INTO repos \
         (name_with_owner, private, archived, fork, language, pushed_at, alerts_enabled, \
          last_scanned_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT (name_with_owner) DO UPDATE SET \
           private = excluded.private, archived = excluded.archived, fork = excluded.fork, \
           language = excluded.language, pushed_at = excluded.pushed_at, \
           alerts_enabled = excluded.alerts_enabled, last_scanned_at = excluded.last_scanned_at",
    )
    .bind(&repo.name_with_owner)
    .bind(repo.private)
    .bind(repo.archived)
    .bind(repo.fork)
    .bind(repo.language.as_deref())
    .bind(repo.pushed_at.map(|instante| instante.to_rfc3339()))
    .bind(alerts_enabled)
    .bind(scanned_at.to_rfc3339())
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Inserta o actualiza una alerta presente en el escaneo y registra su deriva.
///
/// Devuelve la categoría de deriva registrada, o `None` si no había cambio que
/// anotar.
async fn persist_alert(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    repo: &str,
    alert: &Alert,
    run_id: i64,
) -> Result<Option<Drift>, StoreError> {
    let previa: Option<FilaAlerta> = sqlx::query_as(
        "SELECT id, severity, state FROM alerts \
         WHERE source = ?1 AND repo = ?2 AND external_id = ?3",
    )
    .bind(alert.source.as_str())
    .bind(repo)
    .bind(&alert.external_id)
    .fetch_optional(&mut **tx)
    .await?;

    let previous = match previa.as_ref() {
        Some((_, severity, state)) => Some(PriorAlert {
            state: AlertState::parse(state)
                .ok_or_else(|| StoreError::Corrupt(format!("state: {state:?}")))?,
            severity: severity.as_deref().map(Severity::from_github),
        }),
        None => None,
    };

    let Some(cambio) = Drift::classify(previous, Some(severidad_observada(alert))) else {
        return Ok(None);
    };

    match previa {
        Some((id, ..)) => {
            sqlx::query(
                "UPDATE alerts SET ghsa_id = ?1, cve_id = ?2, severity = ?3, cvss = ?4, \
                 summary = ?5, package = ?6, ecosystem = ?7, manifest_path = ?8, scope = ?9, \
                 vulnerable_range = ?10, first_patched = ?11, secret_type = ?12, \
                 created_at = ?13, state = 'open', \
                 last_seen_run = ?14, resolved_at = NULL WHERE id = ?15",
            )
            .bind(alert.ghsa_id.as_deref())
            .bind(alert.cve_id.as_deref())
            .bind(alert.severity.map(Severity::as_str))
            .bind(alert.cvss)
            .bind(alert.summary.as_deref())
            .bind(alert.package.as_deref())
            .bind(alert.ecosystem.as_deref())
            .bind(alert.manifest_path.as_deref())
            .bind(alert.scope.as_deref())
            .bind(alert.vulnerable_range.as_deref())
            .bind(alert.first_patched.as_deref())
            .bind(alert.secret_type.as_deref())
            .bind(alert.created_at.map(|instante| instante.to_rfc3339()))
            .bind(run_id)
            .bind(id)
            .execute(&mut **tx)
            .await?;

            registrar_evento(tx, id, run_id, cambio, AlertState::Open, None).await?;
        }
        None => {
            let insercion = sqlx::query(
                "INSERT INTO alerts \
                 (repo, source, external_id, ghsa_id, cve_id, severity, cvss, summary, package, \
                  ecosystem, manifest_path, scope, vulnerable_range, first_patched, secret_type, \
                  created_at, state, first_seen_run, last_seen_run, resolved_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                  ?16, 'open', ?17, ?17, NULL)",
            )
            .bind(repo)
            .bind(alert.source.as_str())
            .bind(&alert.external_id)
            .bind(alert.ghsa_id.as_deref())
            .bind(alert.cve_id.as_deref())
            .bind(alert.severity.map(Severity::as_str))
            .bind(alert.cvss)
            .bind(alert.summary.as_deref())
            .bind(alert.package.as_deref())
            .bind(alert.ecosystem.as_deref())
            .bind(alert.manifest_path.as_deref())
            .bind(alert.scope.as_deref())
            .bind(alert.vulnerable_range.as_deref())
            .bind(alert.first_patched.as_deref())
            .bind(alert.secret_type.as_deref())
            .bind(alert.created_at.map(|instante| instante.to_rfc3339()))
            .bind(run_id)
            .execute(&mut **tx)
            .await?;

            registrar_evento(
                tx,
                insercion.last_insert_rowid(),
                run_id,
                cambio,
                AlertState::Open,
                None,
            )
            .await?;
        }
    }

    Ok(Some(cambio))
}

/// Marca como resueltas las alertas abiertas que ya no aparecen.
///
/// Solo se consideran los repositorios escaneados con éxito **y** cuya fuente
/// estaba observable: ni un repositorio que falló ni una capacidad no
/// observable provocan falsos `resolved`. Los repositorios que no están en el
/// inventario de esta ejecución tampoco se tocan (limitación conocida: sus
/// alertas no se resuelven). `last_seen_run` **no** se toca: sigue apuntando a
/// la última ejecución que vio la alerta.
async fn resolver_desaparecidas(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    observadas: &HashMap<String, FuentesObservadas>,
    presentes: &HashSet<Clave>,
    run_id: i64,
    now: DateTime<Utc>,
    outcome: &mut PersistOutcome,
) -> Result<(), StoreError> {
    let abiertas: Vec<(i64, String, String, String)> =
        sqlx::query_as("SELECT id, source, repo, external_id FROM alerts WHERE state = 'open'")
            .fetch_all(&mut **tx)
            .await?;

    for (id, source, repo, external_id) in abiertas {
        let Some(fuentes) = observadas.get(&repo) else {
            continue;
        };
        if !fuente_observada(fuentes, &source) {
            continue;
        }
        if presentes.contains(&(source, repo, external_id)) {
            continue;
        }

        sqlx::query("UPDATE alerts SET state = 'resolved', resolved_at = ?1 WHERE id = ?2")
            .bind(now.to_rfc3339())
            .bind(id)
            .execute(&mut **tx)
            .await?;

        registrar_evento(
            tx,
            id,
            run_id,
            Drift::Resolved,
            AlertState::Resolved,
            Some(now),
        )
        .await?;
        outcome.resolved += 1;
    }

    Ok(())
}

/// `true` si la fuente estaba observable en el escaneo del repositorio.
///
/// Dependabot se observa si la capacidad está habilitada; secret scanning, si
/// el colector pudo leerla (`observable`, `false` para `404`/`403`/sin
/// configurar); code scanning, si hay análisis configurado. Un origen
/// desconocido (dato corrupto) nunca declara resueltas sus alertas.
fn fuente_observada(fuentes: &FuentesObservadas, source: &str) -> bool {
    match AlertSource::parse(source) {
        Some(AlertSource::Dependabot) => fuentes.dependabot,
        Some(AlertSource::Secret) => fuentes.secret,
        Some(AlertSource::Code) => fuentes.code,
        None => false,
    }
}

/// Anota un evento de deriva en `alert_events`.
///
/// Guarda, además de la categoría, la **instantánea del ciclo de vida** en ese
/// momento (`state` y `resolved_at`), de modo que el diff de una ejecución sea
/// histórico: no cambia si la alerta se reabre en un run posterior.
async fn registrar_evento(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    alert_id: i64,
    run_id: i64,
    cambio: Drift,
    state: AlertState,
    resolved_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO alert_events (alert_id, run_id, change, state, resolved_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(alert_id)
    .bind(run_id)
    .bind(cambio.as_str())
    .bind(state.as_str())
    .bind(resolved_at.map(|instante| instante.to_rfc3339()))
    .execute(&mut **tx)
    .await?;

    Ok(())
}
