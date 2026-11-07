//! Consultas del histórico.
//!
//! Estas consultas alimentan los subcomandos `history` (serie por ejecución) y
//! `diff` (nuevas y resueltas respecto a la ejecución anterior), que se cablean
//! en el CLI en una fase posterior. [`Store::diff`] resuelve una ejecución
//! concreta y [`Store::latest_diff`] es la variante sin argumentos del CLI.

use chrono::{DateTime, Utc};

use crate::domain::alert::{AlertSource, Severity};
use crate::domain::drift::{AlertState, Drift};
use crate::domain::run::ScanRun;

use super::{Store, StoreError};

/// Fila almacenada de una alerta, en el orden que devuelven las consultas.
type FilaAlmacenada = (
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Fila de la serie histórica: `(run_id, started_at, total, new, resolved)`.
type FilaHistorial = (i64, String, i64, i64, i64);

/// Fila de `scan_runs`.
type FilaRun = (i64, String, String, i64, i64, i64, i64, i64);

/// Fila agregada de `alerts` por repositorio:
/// `(repo, alerts, critical, high, medium, low, unknown)`.
type FilaRecuentoRepo = (String, i64, i64, i64, i64, i64, i64);

/// Alerta almacenada con su ciclo de vida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAlert {
    /// Identificador interno de la alerta.
    pub id: i64,
    /// Repositorio al que pertenece (`owner/name`).
    pub repo: String,
    /// Fuente de la alerta.
    pub source: AlertSource,
    /// Identificador dentro de su fuente.
    pub external_id: String,
    /// Severidad almacenada, si consta.
    pub severity: Option<Severity>,
    /// Estado del ciclo de vida.
    pub state: AlertState,
    /// Ejecución que la vio por primera vez.
    pub first_seen_run: i64,
    /// Última ejecución que la vio.
    pub last_seen_run: i64,
    /// Instante en que quedó resuelta, si lo está.
    pub resolved_at: Option<DateTime<Utc>>,
    /// Tipo de secreto expuesto, si la fuente lo aporta (secret scanning).
    pub secret_type: Option<String>,
    /// Instante de creación de la alerta según la fuente, si consta.
    pub created_at: Option<DateTime<Utc>>,
}

/// Evento de deriva asociado a una ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Alerta a la que se refiere el evento.
    pub alert_id: i64,
    /// Ejecución en la que ocurrió.
    pub run_id: i64,
    /// Categoría de deriva registrada.
    pub change: Drift,
}

/// Punto de la serie histórica: una entrada por ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPoint {
    /// Identificador de la ejecución.
    pub run_id: i64,
    /// Instante de inicio de la ejecución.
    pub started_at: DateTime<Utc>,
    /// Alertas presentes al terminar la ejecución.
    pub total: i64,
    /// Alertas nuevas de esa ejecución.
    pub new: i64,
    /// Alertas resueltas en esa ejecución.
    pub resolved: i64,
}

/// Cambios respecto a una ejecución: nuevas y resueltas, por separado.
///
/// Los campos de ciclo de vida de cada [`StoredAlert`] (`state` y
/// `resolved_at`) son la **instantánea del run consultado**, no el estado
/// actual: el diff es histórico y no cambia si una alerta se reabre después.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// Ejecución de la que se calcula el diff.
    pub run_id: i64,
    /// Alertas que aparecieron en esa ejecución.
    pub new: Vec<StoredAlert>,
    /// Alertas que se resolvieron en esa ejecución.
    pub resolved: Vec<StoredAlert>,
}

/// Recuento de alertas de un repositorio dentro del **conjunto consultado**.
///
/// Los cubos describen el estado y la fuente pedidos en [`RepoQuery`], de modo
/// que no son necesariamente alertas «abiertas».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoAlertCount {
    /// Repositorio (`owner/name`).
    pub repo: String,
    /// Alertas del repositorio en el conjunto consultado.
    pub alerts: i64,
    /// Alertas de severidad crítica.
    pub critical: i64,
    /// Alertas de severidad alta.
    pub high: i64,
    /// Alertas de severidad media.
    pub medium: i64,
    /// Alertas de severidad baja.
    pub low: i64,
    /// Alertas sin severidad conocida (incluye las de severidad nula).
    pub unknown: i64,
}

/// Listado agregado de alertas por repositorio.
///
/// Los recuentos y el desglose describen el **conjunto consultado** (el estado
/// y la fuente pedidos en [`RepoQuery`]), no necesariamente alertas «abiertas».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoAlertList {
    /// Repositorios conocidos (filas de la tabla `repos`), al margen de los filtros.
    pub known_repos: i64,
    /// Repositorios del conjunto con al menos una alerta (los `alerts > 0`).
    pub repos_with_alerts: i64,
    /// Repositorios del conjunto, ya ordenados.
    pub repos: Vec<RepoAlertCount>,
    /// Suma de las alertas del conjunto.
    pub total_alerts: i64,
}

/// Filtros de la agregación de alertas por repositorio.
///
/// `None` en `state` y `source` significa «sin filtro» (todos los estados /
/// todas las fuentes). `min_severity` a `None` equivale a [`Severity::Unknown`]
/// (sin mínimo). `include_empty` añade los repositorios del inventario sin
/// ninguna alerta.
///
/// El [`Default`] derivado equivale a «sin filtro»: todos los estados, todas
/// las fuentes, sin severidad mínima y sin repositorios vacíos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RepoQuery {
    /// Estado de las alertas contadas (`None` = todos).
    pub state: Option<AlertState>,
    /// Fuente de las alertas contadas (`None` = todas).
    pub source: Option<AlertSource>,
    /// Severidad mínima (conserva el repositorio con ≥1 alerta de esa severidad o superior).
    pub min_severity: Option<Severity>,
    /// Incluye los repositorios sin ninguna alerta.
    pub include_empty: bool,
}

impl Store {
    /// Todas las alertas almacenadas, ordenadas por repositorio y fuente.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn alerts(&self) -> Result<Vec<StoredAlert>, StoreError> {
        let filas: Vec<FilaAlmacenada> = sqlx::query_as(
            "SELECT id, repo, source, external_id, severity, state, first_seen_run, \
             last_seen_run, resolved_at, secret_type, created_at FROM alerts \
             ORDER BY repo, source, external_id",
        )
        .fetch_all(self.pool())
        .await?;

        filas.into_iter().map(fila_a_alerta).collect()
    }

    /// Alertas almacenadas de un repositorio concreto.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn alerts_for_repo(&self, repo: &str) -> Result<Vec<StoredAlert>, StoreError> {
        let filas: Vec<FilaAlmacenada> = sqlx::query_as(
            "SELECT id, repo, source, external_id, severity, state, first_seen_run, \
             last_seen_run, resolved_at, secret_type, created_at FROM alerts \
             WHERE repo = ?1 ORDER BY source, external_id",
        )
        .bind(repo)
        .fetch_all(self.pool())
        .await?;

        filas.into_iter().map(fila_a_alerta).collect()
    }

    /// Ejecuciones registradas, en orden cronológico.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn runs(&self) -> Result<Vec<ScanRun>, StoreError> {
        let filas: Vec<FilaRun> = sqlx::query_as(
            "SELECT id, started_at, finished_at, total_repos, ok, failed, alerts_total, \
             schema_version FROM scan_runs ORDER BY id",
        )
        .fetch_all(self.pool())
        .await?;

        filas.into_iter().map(fila_a_run).collect()
    }

    /// Eventos de deriva de una ejecución, en orden de inserción.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn events_for_run(&self, run_id: i64) -> Result<Vec<StoredEvent>, StoreError> {
        let filas: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT alert_id, run_id, change FROM alert_events WHERE run_id = ?1 ORDER BY id",
        )
        .bind(run_id)
        .fetch_all(self.pool())
        .await?;

        filas
            .into_iter()
            .map(|(alert_id, run_id, change)| {
                Ok(StoredEvent {
                    alert_id,
                    run_id,
                    change: Drift::parse(&change)
                        .ok_or_else(|| StoreError::Corrupt(format!("change: {change:?}")))?,
                })
            })
            .collect()
    }

    /// Serie histórica: una entrada por ejecución con total, nuevas y resueltas.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn history(&self) -> Result<Vec<HistoryPoint>, StoreError> {
        let filas: Vec<FilaHistorial> = sqlx::query_as(
            "SELECT r.id, r.started_at, r.alerts_total, \
               (SELECT COUNT(*) FROM alert_events e WHERE e.run_id = r.id AND e.change = 'new'), \
               (SELECT COUNT(*) FROM alert_events e \
                WHERE e.run_id = r.id AND e.change = 'resolved') \
             FROM scan_runs r ORDER BY r.id",
        )
        .fetch_all(self.pool())
        .await?;

        filas
            .into_iter()
            .map(|(run_id, started_at, total, new, resolved)| {
                Ok(HistoryPoint {
                    run_id,
                    started_at: parse_instante(&started_at, "started_at")?,
                    total,
                    new,
                    resolved,
                })
            })
            .collect()
    }

    /// Cambios de una ejecución: las alertas nuevas y las resueltas, separadas.
    ///
    /// El `state` y el `resolved_at` de cada alerta son los que tenía **en esa
    /// ejecución** (la instantánea guardada en `alert_events`), no los actuales:
    /// consultar el diff de un run pasado devuelve el mismo resultado aunque la
    /// alerta se haya reabierto después.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn diff(&self, run_id: i64) -> Result<Diff, StoreError> {
        Ok(Diff {
            run_id,
            new: self.alerts_de_evento(run_id, Drift::New).await?,
            resolved: self.alerts_de_evento(run_id, Drift::Resolved).await?,
        })
    }

    /// Diff del **último** run: los cambios respecto a la ejecución anterior.
    ///
    /// Es la variante sin argumentos que expone el CLI `ghwatch diff` («cambios
    /// desde el último run»): envuelve [`Store::latest_run_id`] y [`Store::diff`].
    /// Devuelve `None` cuando todavía no se ha almacenado ninguna ejecución, de
    /// modo que el llamante distingue «sin histórico» de «sin cambios».
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta o hay un valor corrupto.
    pub async fn latest_diff(&self) -> Result<Option<Diff>, StoreError> {
        match self.latest_run_id().await? {
            Some(run_id) => Ok(Some(self.diff(run_id).await?)),
            None => Ok(None),
        }
    }

    /// Identificador de la última ejecución, si existe.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta.
    pub async fn latest_run_id(&self) -> Result<Option<i64>, StoreError> {
        let id = sqlx::query_scalar("SELECT id FROM scan_runs ORDER BY id DESC LIMIT 1")
            .fetch_optional(self.pool())
            .await?;
        Ok(id)
    }

    /// Alertas que registraron un cambio concreto en una ejecución.
    ///
    /// El `state` y el `resolved_at` se leen del **evento** (instantánea del run
    /// consultado), no de la fila actual de `alerts`.
    async fn alerts_de_evento(
        &self,
        run_id: i64,
        cambio: Drift,
    ) -> Result<Vec<StoredAlert>, StoreError> {
        let filas: Vec<FilaAlmacenada> = sqlx::query_as(
            "SELECT a.id, a.repo, a.source, a.external_id, a.severity, e.state, \
             a.first_seen_run, a.last_seen_run, e.resolved_at, a.secret_type, a.created_at \
             FROM alert_events e JOIN alerts a ON a.id = e.alert_id \
             WHERE e.run_id = ?1 AND e.change = ?2 ORDER BY a.repo, a.source, a.external_id",
        )
        .bind(run_id)
        .bind(cambio.as_str())
        .fetch_all(self.pool())
        .await?;

        filas.into_iter().map(fila_a_alerta).collect()
    }

    /// Recuento de alertas por repositorio según los filtros de [`RepoQuery`].
    ///
    /// Parte del **inventario** (`repos`) con un `LEFT JOIN` sobre `alerts`, de
    /// modo que `include_empty` puede devolver repositorios sin ninguna alerta.
    /// Devuelve todos los repositorios que pasan los filtros, ordenados por
    /// número de alertas descendente y, a igualdad, por nombre ascendente (el
    /// desempate es explícito en el `ORDER BY`, no se delega en el plan de
    /// ejecución). `min_severity` conserva solo los repositorios con al menos
    /// una alerta de esa severidad o superior, sin recortar el recuento ni el
    /// desglose que se devuelven de cada repositorio.
    ///
    /// Precondición: toda fila de `alerts` apunta a un repositorio presente en
    /// `repos` (el esquema no impone clave foránea y `persist` nunca borra de
    /// `repos`). Una alerta huérfana quedaría fuera del listado sin aviso.
    ///
    /// El `LEFT JOIN` introduce tres trampas que la consulta evita de forma
    /// explícita:
    ///
    /// 1. El recuento es `COUNT(a.id)`, **no** `COUNT(*)`: la fila sin
    ///    coincidencia del `LEFT JOIN` tiene `a.*` nulo y `COUNT(*)` daría 1.
    /// 2. El cubo `unknown` exige `a.id IS NOT NULL`: sin ese guardián, la fila
    ///    sin coincidencia (`severity IS NULL`) contaría como desconocida y se
    ///    rompería el invariante `critical+high+medium+low+unknown == alerts`.
    /// 3. El `HAVING` envuelve el `MAX(...)` en `COALESCE(..., 0)`: sin
    ///    coincidencia `MAX` es `NULL` y `NULL >= ?` es `NULL` (falso), así que
    ///    un repositorio vacío no pasaría el filtro cuando no hay severidad
    ///    mínima.
    ///
    /// La instantánea se compone de **dos lecturas** (la agregación y el
    /// `COUNT(*)` de `repos`) que **no comparten transacción**: entre ambas
    /// podría colarse una escritura. En un CLI de solo lectura local el riesgo
    /// es despreciable y no justifica envolverlas en una transacción.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la consulta.
    pub async fn repo_alert_counts(&self, query: &RepoQuery) -> Result<RepoAlertList, StoreError> {
        // Los literales de severidad de esta consulta y el mapeo 4/3/2/1/0 del
        // `HAVING` **deben** coincidir con `Severity::as_str()` y
        // `Severity::rank()` (src/domain/alert.rs): no los hagas divergir. El
        // bucket `unknown` recoge cualquier valor fuera del conjunto (y `NULL`),
        // porque `severity` es `TEXT` sin `CHECK` y podría albergar una variante
        // ajena: así el invariante `critical+high+medium+low+unknown == alerts`
        // se mantiene pase lo que pase. El guardián `a.id IS NOT NULL` evita que
        // la fila nula del `LEFT JOIN` (repositorio sin alertas) cuente como
        // desconocida, y `COUNT(a.id)` evita que cuente como una alerta.
        let filas: Vec<FilaRecuentoRepo> = sqlx::query_as(
            "SELECT r.name_with_owner AS repo, \
               COUNT(a.id) AS alerts, \
               SUM(CASE WHEN a.severity = 'critical' THEN 1 ELSE 0 END), \
               SUM(CASE WHEN a.severity = 'high' THEN 1 ELSE 0 END), \
               SUM(CASE WHEN a.severity = 'medium' THEN 1 ELSE 0 END), \
               SUM(CASE WHEN a.severity = 'low' THEN 1 ELSE 0 END), \
               SUM(CASE WHEN a.id IS NOT NULL \
                 AND (a.severity IS NULL \
                      OR a.severity NOT IN ('critical','high','medium','low')) \
                 THEN 1 ELSE 0 END) \
             FROM repos r \
             LEFT JOIN alerts a \
                    ON a.repo = r.name_with_owner \
                   AND (?1 IS NULL OR a.state = ?1) \
                   AND (?2 IS NULL OR a.source = ?2) \
             GROUP BY r.name_with_owner \
             HAVING (?3 = 1 OR COUNT(a.id) > 0) \
                AND COALESCE(MAX(CASE a.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 \
                                        WHEN 'medium' THEN 2 WHEN 'low' THEN 1 \
                                        ELSE 0 END), 0) >= ?4 \
             ORDER BY COUNT(a.id) DESC, r.name_with_owner ASC",
        )
        .bind(query.state.map(AlertState::as_str))
        .bind(query.source.map(AlertSource::as_str))
        .bind(i64::from(query.include_empty))
        .bind(query.min_severity.map_or(0, Severity::rank))
        .fetch_all(self.pool())
        .await?;

        let repos: Vec<RepoAlertCount> = filas.into_iter().map(fila_a_recuento).collect();
        let total_alerts = repos.iter().map(|repo| repo.alerts).sum();
        let repos_with_alerts =
            i64::try_from(repos.iter().filter(|repo| repo.alerts > 0).count()).unwrap_or(i64::MAX);

        let known_repos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM repos")
            .fetch_one(self.pool())
            .await?;

        Ok(RepoAlertList {
            known_repos,
            repos_with_alerts,
            repos,
            total_alerts,
        })
    }
}

/// Convierte una fila agregada en el recuento de un repositorio.
fn fila_a_recuento(fila: FilaRecuentoRepo) -> RepoAlertCount {
    let (repo, alerts, critical, high, medium, low, unknown) = fila;

    RepoAlertCount {
        repo,
        alerts,
        critical,
        high,
        medium,
        low,
        unknown,
    }
}

/// Convierte una fila almacenada en una alerta, validando sus valores cerrados.
fn fila_a_alerta(fila: FilaAlmacenada) -> Result<StoredAlert, StoreError> {
    let (
        id,
        repo,
        source,
        external_id,
        severity,
        state,
        first_seen_run,
        last_seen_run,
        resolved_at,
        secret_type,
        created_at,
    ) = fila;

    Ok(StoredAlert {
        id,
        repo,
        source: AlertSource::parse(&source)
            .ok_or_else(|| StoreError::Corrupt(format!("source: {source:?}")))?,
        external_id,
        severity: severity.as_deref().map(Severity::from_github),
        state: AlertState::parse(&state)
            .ok_or_else(|| StoreError::Corrupt(format!("state: {state:?}")))?,
        first_seen_run,
        last_seen_run,
        resolved_at: resolved_at
            .as_deref()
            .map(|valor| parse_instante(valor, "resolved_at"))
            .transpose()?,
        secret_type,
        created_at: created_at
            .as_deref()
            .map(|valor| parse_instante(valor, "created_at"))
            .transpose()?,
    })
}

/// Convierte una fila de `scan_runs` en una instantánea.
fn fila_a_run(fila: FilaRun) -> Result<ScanRun, StoreError> {
    let (id, started_at, finished_at, total_repos, ok, failed, alerts_total, schema_version) = fila;

    Ok(ScanRun {
        id,
        started_at: parse_instante(&started_at, "started_at")?,
        finished_at: parse_instante(&finished_at, "finished_at")?,
        total_repos,
        ok,
        failed,
        alerts_total,
        schema_version,
    })
}

/// Interpreta una marca de tiempo almacenada en RFC 3339 (UTC).
fn parse_instante(valor: &str, campo: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(valor)
        .map(|instante| instante.with_timezone(&Utc))
        .map_err(|error| StoreError::Corrupt(format!("{campo}: {valor:?} ({error})")))
}
