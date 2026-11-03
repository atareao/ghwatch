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
- [ ] 2.8 Auditoría con `@rust-reviewer` del cliente GitHub.

## 3. Inventario y recolección

- [ ] 3.1 RED — escribir el test con `wiremock` del descubrimiento de 433 repositorios repartidos en 5 páginas que debe fallar porque no existe el inventario; comando `cargo test inventory`; pegar la salida en rojo.
- [ ] 3.2 RED — escribir el test con `wiremock` de los filtros por defecto (367 de 433) y de `--include-forks` y `--include-archived`; comando `cargo test inventory`; pegar la salida en rojo.
- [ ] 3.3 RED — escribir el test de normalización de una alerta de Dependabot (112 en 2 páginas, sin truncar en 100) al modelo único; comando `cargo test alerts`; pegar la salida en rojo.
- [ ] 3.4 RED — escribir el test de normalización de una alerta de secret scanning con `source = secret` y `cvss` vacío sin error; comando `cargo test alerts`; pegar la salida en rojo.
- [ ] 3.5 RED — escribir el test con `wiremock` de code scanning `404` `no analysis found` que marca «no configurado» y no aborta; comando `cargo test alerts`; pegar la salida en rojo.
- [ ] 3.6 RED — escribir el test con `wiremock` de Dependabot `403` que marca `disabled` y no cuenta como fallo; comando `cargo test alerts`; pegar la salida en rojo.
- [ ] 3.7 GREEN — implementar la recolección de las tres fuentes y la normalización al modelo único; confirmar con `cargo test alerts` y `cargo check`.
- [ ] 3.8 RED — escribir el test de orquestación con concurrencia acotada de 367 repositorios y 2 fallos transitorios que espera 365 correctos y 2 fallidos listados; comando `cargo test scan`; pegar la salida en rojo.
- [ ] 3.9 GREEN — implementar la orquestación con concurrencia acotada y progreso con `indicatif`; confirmar con `cargo test scan` y `cargo check`.
- [ ] 3.10 Auditoría con `@rust-reviewer` del inventario, la recolección y la orquestación.

## 4. Persistencia e histórico

- [ ] 4.1 RED — escribir el test de migración que espera las 4 tablas (`repos`, `scan_runs`, `alerts`, `alert_events`) con la clave única `(source, repo, external_id)` y que debe fallar porque no hay migraciones; comando `cargo test store`; pegar la salida en rojo.
- [ ] 4.2 GREEN — implementar las migraciones y los repositorios de datos sobre sqlx; confirmar con `cargo test store` y `cargo check`.
- [ ] 4.3 RED — escribir el test de idempotencia (dos escaneos idénticos → 0 nuevas, 0 resueltas, `first_seen_run` intacto) que debe fallar; comando `cargo test store`; pegar la salida en rojo.
- [ ] 4.4 RED — escribir el test del ciclo de vida (`first_seen_run`, `last_seen_run`, `resolved_at`) que debe fallar; comando `cargo test store`; pegar la salida en rojo.
- [ ] 4.5 RED — escribir los tests de clasificación de deriva (`new`, `still_open`, `resolved`, `reopened`, `severity_up`) que deben fallar; comando `cargo test drift`; pegar la salida en rojo.
- [ ] 4.6 GREEN — implementar el ciclo de vida y el cálculo de deriva; confirmar con `cargo test store drift` y `cargo check`.
- [ ] 4.7 RED — escribir los tests de `history` (serie por ejecución) y `diff` (nuevas y resueltas separadas) que deben fallar; comando `cargo test history`; pegar la salida en rojo.
- [ ] 4.8 GREEN — implementar los subcomandos `history` y `diff`; confirmar con `cargo test history` y `cargo check`.
- [ ] 4.9 Auditoría con `@rust-reviewer` de la persistencia, la deriva y el histórico.

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
