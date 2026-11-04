# Tasks: ghwatch-scanner

> **Todas las casillas empiezan sin marcar. No se escribe ni una línea de código ni un solo test antes de que el usuario apruebe este change.** El trabajo empieza por la sección 0, que es un punto de parada obligatorio.

## 0. Aprobación (punto de parada del protocolo)

- [x] 0.1 Presentar este change (proposal + specs + design + tasks) y esperar aprobación explícita antes de tocar código.

**Punto de parada obligatorio.** Ninguna casilla de las fases 1 a 6 ni de la fase 7 se marca hasta que exista una aprobación explícita del usuario sobre el conjunto proposal + specs + design + tasks. Sin esa aprobación no se crea el repositorio, ni se añaden dependencias, ni se escriben tests ni código. Marcar cualquier casilla posterior sin esta aprobación invalida el change.

## 1. Andamiaje y autenticación

- [x] 1.1 GREEN — `cargo init --name ghwatch` con Rust edition 2024; confirmar con `cargo check`.
- [x] 1.2 GREEN — declarar dependencias (tokio, clap derive, reqwest con rustls, serde/serde_json, sqlx sqlite runtime-tokio, chrono, anyhow, thiserror, tracing, indicatif, comfy-table) y `wiremock` como dev-dependency; confirmar con `cargo check`.
- [x] 1.3 GREEN — añadir `[lints.clippy] all = "deny"` y `rustfmt.toml`; confirmar con `cargo clippy -- -D warnings` y `cargo fmt --check`.
- [x] 1.4 RED — escribir el test de `auth::resolve_token` con `GITHUB_TOKEN` presente que debe fallar porque el módulo `auth` no existe; comando `cargo test auth`; pegar la salida en rojo.
- [x] 1.5 RED — escribir el test con `GITHUB_TOKEN` ausente y `gh` simulado que debe fallar porque no hay resolución por `gh auth token`; comando `cargo test auth`; pegar la salida en rojo.
- [x] 1.6 RED — escribir el test sin credenciales que espera un error tipado que menciona las dos vías y no contiene valor de token; comando `cargo test auth`; pegar la salida en rojo.
- [x] 1.7 GREEN — implementar el módulo `auth` con el orden `GITHUB_TOKEN` → `gh auth token` → error tipado, sin registrar el token; confirmar con `cargo test auth` y `cargo check`.
- [x] 1.8 Auditoría con `@rust-reviewer` del andamiaje y del módulo `auth`.

### 1.9 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la Fase 1 (veredicto: aprobado con reservas, cero bloqueantes). Todos aplicados con TDD (rojo → verde).

- [x] 1.9.1 Punto 1 — sustituir la aserción anti-fuga tautológica por un test real (estructura local `Envoltorio` con `Debug` que contiene un `Token`; comprobar que el `Debug` de la estructura y el `Debug`/`Display` del error no filtran el secreto). Verificación: `cargo test auth`.
- [x] 1.9.2 Punto 2 — extraer la interpretación de la salida de `gh` a la función pura `interpret_gh_exit(code, stdout)` y cubrir sus ramas (éxito, código≠0, código ausente, vacío, solo espacios, no-UTF-8). Verificación: `cargo test auth`.
- [x] 1.9.3 Punto 3 — corregir la cobertura fantasma: test real con salida vacía (`Ok("   ")`), renombrar el caso que inyectaba `Err(GhEmpty)`, y cubrir el fallthrough de `GITHUB_TOKEN` vacío/en blanco y el recorte del entorno. Verificación: `cargo test auth`.
- [x] 1.9.4 Punto 4 — sustituir `String::from_utf8_lossy` por conversión estricta `String::from_utf8` con la nueva variante `AuthError::GhOutputNotUtf8`. Verificación: `cargo test auth`.
- [x] 1.9.5 Punto 5 — añadir la vía asíncrona `resolve_token_async()` con `tokio::process::Command` y su núcleo `resolve_from_async`, compartiendo la lógica de decisión y la interpretación con la vía síncrona. Verificación: `cargo test auth`.
- [x] 1.9.6 Punto 6 — endurecer `Token` con `Token::new()` (rechaza vacío/solo espacios, `AuthError::EmptyToken`) y eliminar los `impl From<String>`/`From<&str>` públicos. Verificación: `cargo test auth`.
- [x] 1.9.7 Punto 7 — `impl FnOnce` en `resolve_token_from`, comentario sobre `VarError::NotUnicode` y reescritura del doc de módulo (exposición solo vía `expose_secret` y uso síncrono/asíncrono). Verificación: `cargo test`, `cargo clippy --all-targets -- -D warnings` y `cargo fmt --all --check`.

## 2. Cliente GitHub

- [x] 2.1 RED — escribir el test con `wiremock` de una colección de 2 páginas servida por `Link` que debe fallar porque no hay paginación; comando `cargo test github`; pegar la salida en rojo.
- [x] 2.2 RED — escribir el test con `wiremock` que verifica las cabeceras `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28` y `User-Agent: ghwatch`; comando `cargo test github`; pegar la salida en rojo.
- [x] 2.3 RED — escribir el test con `wiremock` de `401` que espera error tipado de autenticación; comando `cargo test github`; pegar la salida en rojo.
- [x] 2.4 RED — escribir el test con `wiremock` de `403` con `Retry-After: 30` que espera espera y reintento, y error tipado al superar el máximo; comando `cargo test github`; pegar la salida en rojo.
- [x] 2.5 RED — escribir el test con `wiremock` de `404` `no analysis found` que espera resultado «no configurado» en lugar de error; comando `cargo test github`; pegar la salida en rojo.
- [x] 2.6 RED — escribir el test con `wiremock` de `X-RateLimit-Remaining` por debajo del umbral que debe fallar porque no hay pausa preventiva; comando `cargo test github`; pegar la salida en rojo.
- [x] 2.7 GREEN — implementar el cliente GitHub con cabeceras, paginación por `Link`, control de rate limit, reintentos con `Retry-After` y concurrencia acotada; confirmar con `cargo test github` y `cargo check`.
- [x] 2.8 Auditoría con `@rust-reviewer` del cliente GitHub.

### 2.9 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la Fase 2 (el cliente HTTP). Un bloqueante (B1) y los hallazgos S1–S6 y S9, todos aplicados con TDD (rojo → verde).

- [x] 2.9.1 B1 (bloqueante) — el test de paginación pasa a usar un `Link` con **forma real de cursor** (otro prefijo de ruta y `after=`), y comprueba que la segunda petición va **exactamente** a la URL del `Link`. Demostrado discriminante: recomponer la URL en lugar de seguir el `Link` crudo hace fallar el test (404 acotado). La página 1 se sirve con `up_to_n_times(1)` para que la recomposición falle en vez de colgarse. Verificación: `cargo test github::paginacion_sigue`.
- [x] 2.9.2 S1 — nueva variante `GithubError::Disabled`, clasificada por el cuerpo real mediante la función pura `is_disabled_body`: un `403` con el literal de Dependabot deshabilitado es `Disabled`; un `403` con otro cuerpo sigue siendo `Http{403}`; un `403` con `Retry-After` sigue siendo `RateLimited` (el límite manda). Verificación: `cargo test github::error`, `cargo test github`.
- [x] 2.9.3 S2 — nuevo campo `GithubConfig::max_pages` (por defecto 1000) y detección de URL repetida (`HashSet` de visitadas); ambos abortan con `GithubError::PaginationLimit` en lugar de colgar el escaneo. Verificación: `cargo test github::paginacion`.
- [x] 2.9.4 S3 — la clasificación solo acepta `2xx` como éxito: un `3xx` (incluido `304`) se devuelve como `Http{status}` en lugar de intentar decodificar un cuerpo que no es el JSON esperado. Verificación: `cargo test github::error`.
- [x] 2.9.5 S4 — `for_each_concurrent` cambia de firma a `Vec<Result<O, JoinError>>`: los fallos de runtime de una tarea ya no se descartan, se devuelven en su posición (el `Id` de la tarea se guarda al lanzarla para mapear el error a su índice). Verificación: `cargo test github::for_each_concurrent`.
- [x] 2.9.6 S5 — test de concurrencia no frágil (sin temporizadores): con un pico medido con `AtomicUsize` y `yield_now`, se comprueba que la concurrencia observada no supera el límite y llega a superar uno; demostrado discriminante (quitar el permiso del semáforo → pico 40 > 6 → rojo). Se documenta que el semáforo solo actúa vía `for_each_concurrent` y que quien lance peticiones por su cuenta debe usar `limiter()`. Verificación: `cargo test github::for_each_concurrent`.
- [x] 2.9.7 S6 — se documenta y se testea que la espera conservadora (`FALLBACK_PAUSE`) solo se aplica cuando consta una lectura de `Remaining` por debajo del umbral: sin `Remaining` legible no se pausa aunque haya `Reset`. Verificación: `cargo test github::ratelimit`.
- [x] 2.9.8 S9 — `with_per_page` analiza la consulta como pares `clave=valor`, de modo que no da falso positivo con `otro=per_page=3` ni con `not_per_page=1`. Verificación: `cargo test github::with_per_page`.

## 3. Inventario y recolección

- [x] 3.1 RED — escribir el test con `wiremock` del descubrimiento de 433 repositorios repartidos en 5 páginas que debe fallar porque no existe el inventario; comando `cargo test inventory`; pegar la salida en rojo.
- [x] 3.2 RED — escribir el test con `wiremock` de los filtros por defecto (367 de 433) y de `--include-forks` y `--include-archived`; comando `cargo test inventory`; pegar la salida en rojo.
- [x] 3.3 RED — escribir el test de normalización de una alerta de Dependabot (112 en 2 páginas, sin truncar en 100) al modelo único; comando `cargo test alerts`; pegar la salida en rojo.
- [x] 3.4 RED — escribir el test de normalización de una alerta de secret scanning con `source = secret` y `cvss` vacío sin error; comando `cargo test alerts`; pegar la salida en rojo.
- [x] 3.5 RED — escribir el test con `wiremock` de code scanning `404` `no analysis found` que marca «no configurado» y no aborta; comando `cargo test alerts`; pegar la salida en rojo.
- [x] 3.6 RED — escribir el test con `wiremock` de Dependabot `403` que marca `disabled` y no cuenta como fallo; comando `cargo test alerts`; pegar la salida en rojo.
- [x] 3.7 GREEN — implementar la recolección de las tres fuentes y la normalización al modelo único; confirmar con `cargo test alerts` y `cargo check`.
- [x] 3.8 RED — escribir el test de orquestación con concurrencia acotada de 367 repositorios y 2 fallos transitorios que espera 365 correctos y 2 fallidos listados; comando `cargo test scan`; pegar la salida en rojo.
- [x] 3.9 GREEN — implementar la orquestación con concurrencia acotada y progreso con `indicatif`; confirmar con `cargo test scan` y `cargo check`.
- [x] 3.10 Auditoría con `@rust-reviewer` del inventario, la recolección y la orquestación.

### 3.11 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la Fase 3 (veredicto: aprobado con reservas, cero bloqueantes). Todos aplicados con TDD (rojo → verde).

- [x] 3.11.1 I1 — `secret_scanning::collect` tolera el `404` que GitHub documenta para repos públicos o con la capacidad desactivada: `NotFound`, `NotConfigured` y `Disabled` se traducen a lista vacía en lugar de contabilizar un fallo. Verificación: `cargo test --test alerts secret_scanning_404` (rojo: `NotFound`).
- [x] 3.11.2 I2 — test del `403` genérico a nivel de colector: `dependabot::collect` contra un `403` con cuerpo ajeno propaga `GithubError::Http { status: 403 }` y no lo confunde con `Disabled`. Verificación: `cargo test --test alerts dependabot_403_generico`.
- [x] 3.11.3 M2 — la marca `no analysis found` se reconoce con la nueva función pura `is_not_configured_body` (insensible a mayúsculas), coherente con `is_disabled_body`. Verificación: `cargo test --lib is_not_configured_body` y `cargo test --lib classify_response_404_marcador` (rojo: `No Analysis Found` clasificado como `NotFound`).
- [x] 3.11.4 M3 — comentario en `code_scanning::collect` que justifica la reutilización de `GithubError::Disabled` (mismo significado operativo: capacidad no disponible) y test del `403` deshabilitado → `configured = false`. Verificación: `cargo test --test alerts code_scanning_403_disabled`.
- [x] 3.11.5 M5 — el paquete se resuelve campo a campo (`security_vulnerability.package.name` → `dependency.package.name` y análogo para `ecosystem`), de modo que un `name` ausente en la vulnerabilidad no oculta el de la dependencia. Verificación: `cargo test --lib dependabot::tests` (rojo: `None` en lugar de `Some("django")`).
- [x] 3.11.6 M6 — cobertura multi-página con `Link` en secret scanning y code scanning: 2 páginas, 101 alertas, sin truncar. Verificación: `cargo test --test alerts recolecta_dos_paginas`.
- [x] 3.11.7 M4 — la spec del modelo único enumera `secret_type`, eliminando la deriva con `src/domain/alert.rs`. Verificación: revisión de `openspec/changes/ghwatch-scanner/specs/alert-collection/spec.md`.

### 3.12 Endurecimiento tras la auditoría de la orquestación

> Cambios derivados de la auditoría de la orquestación (`src/scan.rs`, `tests/scan.rs`, tareas 3.8–3.9; veredicto: aprobado con reservas, cero bloqueantes). Todos aplicados con TDD (rojo → verde).

- [x] 3.12.1 I1 — la salida de cada tarea viaja con su propio nombre de repositorio (`RepoOutcome`): la atribución de un fallo ya no se re-deriva por posición ni puede degradar en silencio a un nombre vacío. La ruta `Err(JoinError)`, sin identidad, se etiqueta por posición (`<repositorio #N>`), nunca vacía. Verificación: `cargo test --lib el_panico_de_una_tarea_no_deja_nombre_vacio` (rojo: `RepoOutcome` inexistente) y `cargo test --test scan escaneo_con_todos_los_repositorios_fallidos` (afirma nombre no vacío).
- [x] 3.12.2 M1 — nueva variante `GithubError::TaskFailed(String)` con accesor `is_task_failed()`: el `JoinError` de un pánico se conserva como causa en lugar de sobrecargar `GithubError::Transport` (que significa otra cosa). Verificación: `cargo test --lib is_task_failed` (rojo: variante inexistente).
- [x] 3.12.3 M2 — el progreso avanza en la ruta de pánico mediante `ProgresoGuard` (el `Drop` corre durante el desenrollado), no solo tras la recolección. Verificación: `cargo test --lib el_progreso_avanza_aunque_la_tarea_entre_en_panico` (rojo: `ProgresoGuard` inexistente).
- [x] 3.12.4 M3 — `collect_repo` acepta el `Repo` por valor: la clausura ya es dueña, así que no se clona el metadato. Verificación: `cargo test --lib scan`.
- [x] 3.12.5 M4 — `RepoScan` expone el estado de Dependabot por una sola vía (`repo.dependabot`); las alertas pasan a `dependabot_alerts`, eliminando la copia del estado que podía desincronizarse. Verificación: `cargo test --test scan escaneo_fusiona_el_estado_de_dependabot` (rojo: campo `dependabot_alerts` inexistente).
- [x] 3.12.6 M5 — cobertura de huecos: descubrimiento vacío (0 repos → resumen vacío y `start(0)`, sin error), todos los repositorios fallan, ruta `Err(JoinError)`, y el escenario de fallos transitorios afirma además que el error es de límite (`is_rate_limited()`). Verificación: `cargo test --test scan` (rojo: `RepoOutcome`/`TaskFailed` inexistentes).

## 4. Persistencia e histórico

- [x] 4.1 RED — escribir el test de migración que espera las 4 tablas (`repos`, `scan_runs`, `alerts`, `alert_events`) con la clave única `(source, repo, external_id)` y que debe fallar porque no hay migraciones; comando `cargo test store`; pegar la salida en rojo.
- [x] 4.2 GREEN — implementar las migraciones y los repositorios de datos sobre sqlx; confirmar con `cargo test store` y `cargo check`.
- [x] 4.3 RED — escribir el test de idempotencia (dos escaneos idénticos → 0 nuevas, 0 resueltas, `first_seen_run` intacto) que debe fallar; comando `cargo test store`; pegar la salida en rojo.
- [x] 4.4 RED — escribir el test del ciclo de vida (`first_seen_run`, `last_seen_run`, `resolved_at`) que debe fallar; comando `cargo test store`; pegar la salida en rojo.
- [x] 4.5 RED — escribir los tests de clasificación de deriva (`new`, `still_open`, `resolved`, `reopened`, `severity_up`) que deben fallar; comando `cargo test drift`; pegar la salida en rojo.
- [x] 4.6 GREEN — implementar el ciclo de vida y el cálculo de deriva; confirmar con `cargo test store drift` y `cargo check`.
- [x] 4.7 RED — escribir los tests de `history` (serie por ejecución) y `diff` (nuevas y resueltas separadas) que deben fallar; comando `cargo test history`; pegar la salida en rojo. En `tests/history.rs` (módulo `history`), con SQLite en memoria: serie de cinco ejecuciones (fecha/total/nuevas/resueltas por run), diff con nuevas y resueltas separadas, y los límites «sin ejecuciones → sin diff» y «run sin cambios → 0/0». Rojo real: `error[E0599]: no method named 'latest_diff' found for struct 'Store'` (la spec del CLI pide `diff` como «cambios desde el último run»; `history` y `diff(run_id)` ya existían de 4.6).
- [x] 4.8 GREEN — implementar los subcomandos `history` y `diff`; confirmar con `cargo test history` y `cargo check`. `Store::latest_diff()` (variante sin argumentos: `latest_run_id` + `diff`, `None` sin histórico) sobre la serie `Store::history()` y la primitiva `Store::diff(run_id)` ya existentes. Verde: `cargo test history` (4 tests) y `cargo test` (144 tests). El cableado del subcomando en el CLI queda para la Fase 7.
- [x] 4.9 Auditoría con `@rust-reviewer` de la persistencia, la deriva y el histórico.

### 4.10 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la Fase 4 (veredicto: aprobado con reservas, cero bloqueantes). Todos aplicados con TDD (rojo → verde).

- [x] 4.10.1 I1 — `secret_type` se persiste y se recupera: nueva columna `secret_type TEXT` en `alerts`, vinculada en el INSERT/UPDATE de `persist.rs` y expuesta en `StoredAlert` (`secret_type`). `PLAN.md` §4 actualizado. Verificación: `cargo test --test store una_alerta_de_secreto_persiste` (rojo: `E0609: no field secret_type on type StoredAlert`).
- [x] 4.10.2 I2 — observabilidad de secret scanning: `secret_scanning::collect` devuelve `SecretScanningReport { observable, alerts }` (`observable = false` para `404`/`403`/`NotConfigured`), se propaga hasta `RepoScan.secrets` y `fuente_observada` la usa en lugar de `true` fijo. Verificación: `cargo test --test alerts secret_scanning_404_marca_la_fuente_no_observable` (rojo: `E0432: no SecretScanningReport` y `E0609: no field observable`) y `cargo test --test store no_se_resuelven_secretos` (un `404` sigue sin contar como fallo).
- [x] 4.10.3 I3 — el diff es **histórico** (opción b): `alert_events` guarda la instantánea `state`/`resolved_at` del evento y `alerts_de_evento` los lee del evento, no de la fila actual. Verificación: `cargo test --test history el_diff_de_un_run_no_cambia_al_reabrir` (rojo: `left: Open, right: Resolved`).
- [x] 4.10.4 M1 — índices que faltan: `idx_alerts_state` y el compuesto `idx_alert_events_run_change`. Verificación: `cargo test --test store las_migraciones_crean_los_indices` (rojo: falta `idx_alerts_state`).
- [x] 4.10.5 M3 — limitación conocida documentada (doc de `persist_scan`/`resolver_desaparecidas`, `PLAN.md` §4 y spec `alert-history`): las alertas de repositorios que salen del inventario no se resuelven.
- [x] 4.10.6 M5 — cobertura: mismo `external_id` en dos fuentes del mismo repo (`el_mismo_external_id_en_dos_fuentes`), reapertura posterior al run consultado (`el_diff_de_un_run_no_cambia_al_reabrir`), alerta resuelta ausente varios runs sin alterar `resolved_at` (`una_alerta_resuelta_ausente_conserva_su_resolved_at`) y aserción explícita de `severity_up == 0` en un escaneo repetido.
- [x] 4.10.7 M6 — `Eq` derivado en `Diff` y `StoredAlert`, coherente con `HistoryPoint`/`ScanRun`.
- [x] 4.10.8 Verificación final: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test` (151 tests) en verde.

## 5. Enriquecimiento OSV y priorización

- [ ] 5.1 RED — escribir el test con `wiremock` de 1500 pares que espera exactamente 2 peticiones a `POST /v1/querybatch` y que debe fallar porque no existe el cliente OSV; comando `cargo test osv`; pegar la salida en rojo.
- [ ] 5.2 RED — escribir el test de mapeo de ecosistemas (`pip → PyPI`, etc.) y de la omisión sin error de un ecosistema sin equivalencia; comando `cargo test osv`; pegar la salida en rojo.
- [ ] 5.3 RED — escribir el test de «completar sin sobreescribir» (`first_patched` de GitHub conservado, `cvss` a 0 completado por OSV); comando `cargo test osv`; pegar la salida en rojo.
- [ ] 5.4 RED — escribir el test de fallo de OSV que espera escaneo correcto, alertas sin enriquecer y aviso; comando `cargo test osv`; pegar la salida en rojo.
- [ ] 5.5 RED — escribir el test de priorización determinista (antigüedad y reproducibilidad) que debe fallar; comando `cargo test priority`; pegar la salida en rojo.
- [ ] 5.6 GREEN — implementar el cliente OSV por lotes, la tabla de mapeo, la fusión sin sobreescritura y la priorización; confirmar con `cargo test osv priority` y `cargo check`.
- [ ] 5.7 Auditoría con `@rust-reviewer` del enriquecimiento y la priorización.

## 6. Informes y notificación a Valet

- [ ] 6.1 RED — escribir el test de la tabla de consola agrupada por severidad y ordenada por prioridad descendente que debe fallar; comando `cargo test report`; pegar la salida en rojo.
- [ ] 6.2 RED — escribir el test del Markdown de situación actual (totales, ecosistemas y exactamente 15 repositorios) que debe fallar; comando `cargo test report`; pegar la salida en rojo.
- [ ] 6.3 RED — escribir el test del Markdown de evolución (fecha, nuevas, resueltas, neto y tendencia gráfica) y del bloque de acciones por paquete; comando `cargo test report`; pegar la salida en rojo.
- [ ] 6.4 RED — escribir el test del JSON con `schema_version` y claves estables; comando `cargo test report`; pegar la salida en rojo.
- [ ] 6.5 GREEN — implementar la salida de consola, el Markdown y el JSON; confirmar con `cargo test report` y `cargo check`.
- [ ] 6.6 RED — escribir el test con `wiremock` del envío a Valet (`201` con identificador, `401` abortado sin reintento) que debe fallar; comando `cargo test notify`; pegar la salida en rojo.
- [ ] 6.7 RED — escribir los tests de los límites exactos: `message` de 5000 bytes recortado a ≤4096 sin partir multibyte, metadato ≤8192 bytes que nunca produce `400`, y presencia de la ruta del informe; comando `cargo test notify`; pegar la salida en rojo.
- [ ] 6.8 RED — escribir el test del nivel derivado (`critical`/`warning`/`info`) y del modo silencioso por defecto con `--force`; comando `cargo test notify`; pegar la salida en rojo.
- [ ] 6.9 RED — escribir el test de configuración con permisos distintos de `600` rechazada y de log sin token; comando `cargo test notify`; pegar la salida en rojo.
- [ ] 6.10 GREEN — implementar el notificador a Valet con best-effort, niveles, modo silencioso y `--force`; confirmar con `cargo test notify` y `cargo check`.
- [ ] 6.11 GREEN — conectar la escritura de informes con independencia del resultado de la notificación; confirmar con `cargo test report notify` y `cargo check`.
- [ ] 6.12 Auditoría con `@rust-reviewer` de los informes y la notificación.

## 7. Operación

- [ ] 7.1 GREEN — crear las unidades systemd de usuario (`ghwatch.service` y `ghwatch.timer`) con `Persistent=true`; verificar con `systemd-analyze verify`.
- [ ] 7.2 GREEN — crear el `justfile` con las recetas de build, test, lint y escaneo; verificar ejecutando `just --list`.
- [ ] 7.3 GREEN — redactar `README.md` y `README.es.md`; verificar revisando cobertura de uso, configuración y operación.
- [ ] 7.4 GREEN — configurar la CI con `cargo fmt --check`, `cargo clippy -- -D warnings` y `cargo test`; verificar con `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- [ ] 7.5 GREEN — preparar el flujo gitflow: rama `development`, una rama `feature/*` por fase y commits gitmoji, con PR a `development` (nunca commit directo a `main` ni `development`); verificar con `git branch --show-current` y la revisión de los PRs.
- [ ] 7.6 Auditoría con `@rust-reviewer` de la operación y del repositorio completo.
- [ ] 7.7 Verificación final contra los criterios de aceptación del `PLAN.md`: cubrir 367 repositorios en menos de 15 minutos, detectar las ≈1.333 alertas, idempotencia confirmada, informe con evolución, notificación visible en Valet, `clippy -- -D warnings` limpio y tests verdes sin red.
