# PLAN.md — ghwatch: vigilancia de vulnerabilidades en todos los repos de GitHub

> Checkpoint arquitectónico. Generado tras la fase de reconocimiento (verificado contra la API real el 2026-11-03).

## 1. Objetivo

Proceso automatizable que revisa **todos** los repositorios GitHub de la cuenta `atareao`, detecta
vulnerabilidades y secretos expuestos, mantiene **histórico de evolución**, genera un **informe de
situación actual + evolución**, y opcionalmente **envía el resumen a Valet** (asistente self-hosted).

Binario: `ghwatch` (CLI en Rust). Ubicación: `/home/lorenzo/sandbox/ghwatch`.

## 2. Hallazgos verificados del reconocimiento

| Comprobación | Resultado medido |
|---|---|
| Repos accesibles vía `gh` | 433 totales; **367 activos** (sin forks ni archivados) |
| Token actual: scopes | `admin:public_key, gist, read:org, repo` |
| `GET /repos/{r}/dependabot/alerts` | **200 OK** — legible con scope `repo` |
| `GET /repos/{r}/secret-scanning/alerts` | **200 OK** — legible |
| `GET /repos/{r}/code-scanning/alerts` | 404 `no analysis found` (no configurado; no es problema de permisos) |
| `GET /repos/{r}/dependency-graph/sbom` | 200 OK |
| `GET /repos/{r}/vulnerability-alerts` | 204 (Dependabot habilitado en los 12 repos muestreados) |
| **Alertas Dependabot abiertas** | **≈1.333 en 90 repos** |
| Secret scanning | 3 alertas abiertas ya en `atareao/reto-python` |
| OSV.dev (`POST /v1/querybatch`) | Funciona **sin autenticación**; devuelve CVSS, alias CVE, fix |
| Rate limit | 5000 req/h core · 5000 pts/h GraphQL |
| `gh auth token` | Disponible (token de 40 chars) |
| Valet | Escuchando en `127.0.0.1:3000`, operativo |

### Top repos por alertas abiertas
`reto-python` 112 · `presu` 80 · `bloc` 80 · `tareas` 59 · `asistentes` 55 · `asistente` 54 ·
`quadly` 52 · `cpu-g` 44 · `alloy` 36 · `pixabay-search-provider` 35 · `kvault` 32 · `preemer` 31 ·
`populatrs` 31 · `bmc` 31.

### Tres hallazgos que condicionan la arquitectura
1. **Hay que paginar.** `reto-python` devolvía 100 alertas con `per_page=100` pero tiene **112**.
   Cualquier conteo sin paginación está mal.
2. **El SBOM no basta como fuente primaria.** Para `reto-python` el dependency-graph solo ve 33
   paquetes (manifiestos de raíz) frente a 112 alertas que Dependabot sí detecta en subcarpetas
   (`reto-11/joselo/requirements.txt`). Dependabot API = fuente primaria; SBOM/OSV = complemento.
3. **Sin escáneres locales.** No hay `osv-scanner`, `trivy`, `grype` ni `syft` instalados; la
   API de OSV es la vía sin dependencias externas.

## 3. Decisiones tomadas (confirmadas por el usuario)

- **Formato**: CLI en Rust + informe. Sin dashboard web ni Tauri en esta iteración (la base queda
  preparada para montarlos después consumiendo el JSON).
- **Alcance**: Dependabot alerts + secret scanning + enriquecimiento vía OSV.dev.
- **Notificación**: informe con **situación actual y evolución** en disco, más **envío opcional a
  Valet** (`/data/rust/valet`). Sin issues automáticos en repos, sin Telegram.

## 4. Arquitectura

### Stack
`Rust edition 2024` · `tokio` · `clap` (derive) · `reqwest` (rustls-tls) · `serde`/`serde_json` ·
`sqlx` (sqlite + runtime-tokio, queries en runtime) · `chrono` · `anyhow` + `thiserror` ·
`tracing` + `tracing-subscriber` · `indicatif` (progreso) · `comfy-table` (salida) · `wiremock` (tests).

### Mapa de módulos
```
src/
  main.rs            # entrypoint, init tracing
  cli.rs             # clap: scan | report | diff | history | notify | init
  config.rs          # config.toml + env + limits
  auth.rs            # GITHUB_TOKEN -> `gh auth token` -> error claro
  github/
    client.rs        # reqwest + auth + rate-limit + retry + User-Agent
    pagination.rs    # Link rel="next" -> stream de items
    repos.rs         # descubrimiento de repositorios
    dependabot.rs    # alertas Dependabot
    secret_scanning.rs
    code_scanning.rs # tolerante a 404 "no analysis found"
  osv/client.rs      # querybatch + mapeo de ecosistemas
  store/             # sqlx, migraciones, upserts, cálculo de drift
  domain/            # Alert, Severity, ScanRun, Drift, Priority (tipos puros)
  report/            # table.rs, markdown.rs, json.rs
  notify/            # valet.rs, desktop.rs (opcional)
```
Principio: `domain` es puro y testeable sin red ni BD; el resto depende de él, nunca al revés.

### Modelo de datos (SQLite)
- `repos(id, name_with_owner, private, archived, fork, language, pushed_at, alerts_enabled, last_scanned_at)`
- `scan_runs(id, started_at, finished_at, total_repos, ok, failed, alerts_total, schema_version)`
- `alerts(id, repo, source, external_id, ghsa_id, cve_id, severity, cvss, summary, package,
  ecosystem, manifest_path, scope, vulnerable_range, first_patched, secret_type, created_at,
  state, first_seen_run, last_seen_run, resolved_at)`
- `alert_events(id, alert_id, run_id, change, state, resolved_at)` — `change ∈ {new, still_open, resolved, reopened, severity_up}`;
  `state` y `resolved_at` guardan la **instantánea** del ciclo de vida en el run del evento, de
  modo que el diff de un run es **histórico** y no cambia si la alerta se reabre después.
- Índices: `idx_alerts_repo`, `idx_alerts_state` (resolución de desaparecidas),
  `idx_alert_events_run` y `idx_alert_events_run_change` (diff por run y cambio).
- `UNIQUE(source, repo, external_id)` para idempotencia.
- Limitación conocida: las alertas de repositorios que salen del inventario (nuevos forks,
  archivados o fuera del filtro) no se resuelven, porque no se observan; permanecen abiertas.

### Control de rate limit
Semáforo `tokio` con concurrencia configurable (def. 6) + lectura de
`X-RateLimit-Remaining`/`Reset` (pausa si <100) + retry con `Retry-After` en 403 de límite
secundario, con backoff exponencial y máximo de reintentos. 367 repos × ~3 endpoints ≈ 1100
llamadas: cabe de sobra en 5000/h.

### Mapeo de ecosistemas (Dependabot → OSV)
`pip → PyPI` · `npm → npm` · `cargo → crates.io` · `go → Go` · `composer → Packagist` ·
`maven → Maven` · `nuget → NuGet` · `bundler → RubyGems` · `github-actions → GitHub Actions`.

## 5. Contrato con Valet (verificado en `/data/rust/valet/docs/api-notifications.md`)

Valet ya expone la API exacta que hace falta; **no hay que tocar Valet**.

- Endpoint: `POST {VALET_BASE_URL}/api/notifications` (def. `http://localhost:3000`).
- Auth: `Authorization: Bearer val_<base64url>` (**ingest key**, exenta de sesión OIDC).
  La key se crea con `POST /api/ingest-keys` (requiere sesión) y **solo se muestra una vez**.
- Body JSON: `message` (obligatorio, ≤4096 bytes tras `trim`, no vacío) · `origin` (≤64 B,
  def. nombre de la key) · `author` (≤128 B) · `level` (`info|warning|critical`, exacto, sin trim) ·
  `metadata` (objeto JSON, ≤8192 B serializado).
- Respuesta: `201 {"id": "<uuid>"}`. Errores: 400/401/413/415/500.
- Efecto secundario: Valet también dispara **WebPush** a los navegadores suscritos.

Mapeo de `level`: hay `critical` → `critical`; si no, hay `high` → `warning`; en otro caso `info`.

`metadata` (schema estable `schema_version: 1`): `{schema_version, run_id, scanned, by_severity
{critical,high,medium,low}, new, resolved, worst_repos[5], report_path}`.
El informe completo **nunca** viaja en `message`: se recorta a ≤4096 bytes **por bytes UTF-8**
(no por chars) y se referencia la ruta del informe en disco.

Modo por defecto: solo notifica si `new > 0` o `resolved > 0` (evita el spam
diario). `--force` para forzar el envío aunque no haya novedades. Si Valet está caído: la
notificación es *best-effort*, el informe en disco ya está escrito y el exit code del escaneo no
falla por ello.

## 6. Fases y tareas atómicas

### Fase 1 — Núcleo: escaneo de alertas Dependabot
- **T1.1** Scaffolding: `cargo init`, dependencias, `rustfmt.toml`, `[lints.clippy]` con `all = "deny"`,
  estructura de módulos con stubs compilables.
- **T1.2** `auth`: resolución de token en orden `GITHUB_TOKEN` → `gh auth token` → error explicativo.
- **T1.3** `github/client`: headers (`Accept: application/vnd.github+json`,
  `X-GitHub-Api-Version: 2022-11-28`, `User-Agent: ghwatch`), parseo de rate limit, retry/backoff,
  errores tipados. Tests con `wiremock`: 200, 401, 403+`Retry-After`, 404.
- **T1.4** `github/pagination`: seguir `Link: rel="next"` con `per_page=100`. Test con fixture de
  3 páginas que reproduce el caso 112 vs 100.
- **T1.5** `github/repos`: descubrimiento paginado de repos (367 activos; flags
  `--include-forks`, `--include-archived`).
- **T1.6** `github/dependabot`: alertas `state=open` paginadas, normalizadas a `Alert`; tolerar 403
  (alerts deshabilitado) y 404 sin abortar el run.
- **T1.7** CLI `ghwatch scan`: orquestación con semáforo, progreso con `indicatif`, resumen final.

### Fase 2 — Persistencia y evolución
- **T2.1** Migraciones sqlx: las 4 tablas + índices.
- **T2.2** Repositorio de datos: upsert idempotente, `first_seen_run` / `last_seen_run` / `resolved_at`.
- **T2.3** Cálculo de drift contra el run anterior: `new`, `still_open`, `resolved`, `reopened`, `severity_up`.
- **T2.4** Comandos `ghwatch history` (series) y `ghwatch diff` (cambios desde el último run).
- **T2.5** Tests de drift con datasets sintéticos + test de **idempotencia** (dos scans idénticos →
  0 nuevas, 0 resueltas).

### Fase 3 — Enriquecimiento y secretos
- **T3.1** Cliente OSV (`querybatch`, lotes ≤1000) + tabla de mapeo de ecosistemas; completa CVSS,
  alias CVE y versión que corrige cuando Dependabot no los trae. Test de querybatch con fixture real.
- **T3.2** Secret scanning → `source = secret`, con `secret_type`, sin CVSS.
- **T3.3** Code scanning tolerante: 404 → estado "no configurado", nunca error.
- **T3.4** Priorización determinista: score = f(severidad, CVSS, ¿hay fix?, scope runtime vs
  development, antigüedad). Tests deterministas.

### Fase 4 — Informes e integración con Valet
- **T4.1** Salida tabular con `comfy-table` y color por severidad.
- **T4.2** Markdown: **Situación actual** (totales por severidad, por ecosistema, top 15 repos) +
  **Evolución** (últimos N runs: fecha / nuevas / resueltas / neto, con sparkline) + **Top acciones**
  (paquete → versión que corrige).
- **T4.3** JSON estable y versionado (`schema_version: 1`) para consumo externo.
- **T4.4** Notificador Valet según el contrato de la sección 5. Tests de límites **exactos**
  (4096 y 8192 bytes) y de truncado con multibyte (emoji, acentos) sin partir caracteres.
- **T4.5** Notificación por defecto solo-si-hay-cambios (`new > 0` o `resolved > 0`) y `--force`.
- **T4.6** Doc de integración: crear ingest key, guardar el token (`~/.config/ghwatch/config.toml`,
  permisos 600, o keyring), y curl de verificación contra `GET /api/notifications/unread`.

### Fase 5 — Operación
- **T5.1** Unidades de usuario `systemd/ghwatch.service` + `ghwatch.timer` (diario 07:00,
  `Persistent=true`) + instrucciones de instalación.
- **T5.2** `justfile`: `scan`, `report`, `diff`, `test`, `lint`, `build`, `install-timer`.
- **T5.3** `README.md` + `README.es.md` con quickstart.
- **T5.4** CI GitHub Actions: `cargo fmt --check`, `clippy -- -D warnings`, `cargo test`.
- **T5.5** Gitflow: `git init`, rama `development`, una `feature/*` por fase, commits gitmoji,
  PR a `development` (nunca commit directo a `main` ni `development`).

## 7. Puertas de calidad (obligatorias)

1. **OpenSpec (SDD)**: crear el change `ghwatch-scanner` con `proposal.md`, deltas de spec
   (contratos + escenarios Given/When/Then) y `tasks.md`. **STOP: no se escribe código de
   aplicación ni tests nuevos hasta que el usuario apruebe la spec.**
2. **TDD**: rojo → verde → refactor. Los tests se escriben antes que la implementación y se
   ejecutan por CLI (`cargo test`).
3. **Sin red en tests**: GitHub, OSV y Valet se simulan con `wiremock`.
4. **Higiene**: `cargo fmt --check`, `cargo clippy -- -D warnings` sin warnings, `cargo test` verde.
5. **Revisión**: cada fase la audita `@rust-reviewer` antes de darse por cerrada.

## 8. Criterios de aceptación (Definition of Done)

- [ ] `ghwatch scan` cubre los 367 repos activos en < 15 min sin agotar el rate limit y sin fallos
      no explicados.
- [ ] Clasifica ≥1.333 alertas abiertas por severidad, con paginación correcta (112 reales en
      `reto-python`, no 100).
- [ ] Un segundo scan idéntico produce **0 nuevas y 0 resueltas** (idempotencia demostrada).
- [ ] Informe Markdown con **situación actual** y **evolución** histórica.
- [ ] La notificación llega a Valet: aparece en `GET /api/notifications/unread` y dispara WebPush.
- [ ] `cargo clippy -- -D warnings` limpio y `cargo test` 100% verde.
- [ ] Cero llamadas de red en la suite de tests.
- [ ] Instalación operativa por `systemd --user` timer sin intervención manual.

## 9. Riesgos y mitigaciones

| Riesgo | Mitigación |
|---|---|
| Límite secundario (403) de GitHub | Concurrencia ≤6, respetar `Retry-After`, backoff exponencial |
| 66 repos inactivos (forks/archivados) | Excluidos por defecto; flags para incluirlos |
| Token fine-grained futuro | Necesitaría permisos «Dependabot alerts: read» y «Secret scanning alerts: read» |
| Valet caído | Notificación best-effort; el informe en disco nunca se pierde; no altera el exit code |
| SBOM incompleto (33 vs 112) | El SBOM no es fuente primaria; Dependabot manda |
| `message` > 4096 B | Recorte por bytes UTF-8 + ruta del informe en `metadata` |
| Ingest key solo se muestra una vez | Guardarla al crearla, en fichero 600 o keyring; doc de verificación |

## 10. Fuera de alcance (esta iteración)

Dashboard web / Tauri, apertura automática de issues en repos, notificaciones Telegram,
escaneo local de clones con `osv-scanner`, remediación automática (PRs de actualización).
