-- Esquema inicial de ghwatch: inventario, ejecuciones, alertas y eventos.
--
-- La clave única (source, repo, external_id) es la base de la idempotencia:
-- repetir un escaneo sin cambios no duplica ni altera las alertas existentes.
-- Las marcas de tiempo se guardan como texto RFC 3339 (UTC).

CREATE TABLE IF NOT EXISTS repos (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name_with_owner TEXT    NOT NULL UNIQUE,
    private         INTEGER NOT NULL DEFAULT 0,
    archived        INTEGER NOT NULL DEFAULT 0,
    fork            INTEGER NOT NULL DEFAULT 0,
    language        TEXT,
    pushed_at       TEXT,
    alerts_enabled  INTEGER NOT NULL DEFAULT 0,
    last_scanned_at TEXT
);

CREATE TABLE IF NOT EXISTS scan_runs (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at     TEXT    NOT NULL,
    finished_at    TEXT    NOT NULL,
    total_repos    INTEGER NOT NULL DEFAULT 0,
    ok             INTEGER NOT NULL DEFAULT 0,
    failed         INTEGER NOT NULL DEFAULT 0,
    alerts_total   INTEGER NOT NULL DEFAULT 0,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS alerts (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    repo             TEXT    NOT NULL,
    source           TEXT    NOT NULL,
    external_id      TEXT    NOT NULL,
    ghsa_id          TEXT,
    cve_id           TEXT,
    severity         TEXT,
    cvss             REAL,
    summary          TEXT,
    package          TEXT,
    ecosystem        TEXT,
    manifest_path    TEXT,
    scope            TEXT,
    vulnerable_range TEXT,
    first_patched    TEXT,
    secret_type      TEXT,
    created_at       TEXT,
    state            TEXT    NOT NULL DEFAULT 'open',
    first_seen_run   INTEGER NOT NULL,
    last_seen_run    INTEGER NOT NULL,
    resolved_at      TEXT,
    UNIQUE (source, repo, external_id)
);

CREATE TABLE IF NOT EXISTS alert_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    alert_id    INTEGER NOT NULL REFERENCES alerts (id) ON DELETE CASCADE,
    run_id      INTEGER NOT NULL REFERENCES scan_runs (id) ON DELETE CASCADE,
    change      TEXT    NOT NULL,
    -- Instantánea del ciclo de vida en el momento del evento: el diff de un run
    -- es histórico y no cambia si la alerta se reabre después.
    state       TEXT    NOT NULL,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_alerts_repo ON alerts (repo);
CREATE INDEX IF NOT EXISTS idx_alerts_state ON alerts (state);
CREATE INDEX IF NOT EXISTS idx_alert_events_run ON alert_events (run_id);
CREATE INDEX IF NOT EXISTS idx_alert_events_run_change ON alert_events (run_id, change);
