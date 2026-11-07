# AGENTS.md — ghwatch

CLI en Rust (edición 2024) que vigila las vulnerabilidades en todos los repos GitHub de una
cuenta (Dependabot + secret/code scanning, enriquecido con OSV.dev, con histórico SQLite e
informes). Especificación viva: `openspec/`. Checkpoint arquitectónico: `PLAN.md`.

## Estado actual — leer antes de tocar código

- **CLI completo y operativo.** `src/main.rs` configura `tracing` (siempre a `stderr`) y
  delega en `ghwatch::cli::run`; `src/cli.rs` cablea los siete subcomandos
  `scan|report|diff|history|repos|notify|init` (`repos` lista los repositorios con sus alertas
  abiertas desde la base local) y `src/config.rs` resuelve la configuración con
  precedencia **flags > entorno > fichero > defecto**. Códigos de salida: `0` éxito, `2` uso,
  `3` sin credencial de GitHub, `4` fallo de la API de GitHub y `5` fallo local (base de datos,
  informe o E/S). `cargo install --path . --locked --root "$HOME/.local"` (o `just install`)
  instala el escáner real en `~/.local/bin/ghwatch`, que es lo que espera el unit de systemd.
- Implementado y auditado: `auth`; `github` (cliente HTTP + recolección del inventario y de las
  tres fuentes de alertas); `domain` (tipos puros, con `priority`); `scan` (orquestación con
  concurrencia acotada); `store` (persistencia SQLite, ciclo de vida, deriva y la agregación de
  alertas abiertas por repositorio); `osv` (enriquecimiento best-effort por lotes contra OSV.dev);
  `report` (consola, Markdown y JSON, sin red, incluido el listado de repositorios con alertas);
  `notify` (notificador Valet best-effort + coordinación informe/envío); `cli` (`clap` con los
  siete subcomandos y los códigos de salida) y `config` (config.toml + entorno + precedencia y
  permisos).
- Operación lista: unidades de usuario de systemd (`systemd/ghwatch.service` y `ghwatch.timer`)
  y recetas en `justfile`; la CI (`.github/workflows/ci.yml`) corre fmt → clippy → test.
- El trabajo pasa por el flujo SDD + TDD + auditoría descrito más abajo; no se implementan
  módulos ni se cambia el comportamiento por iniciativa propia.

## Flujo obligatorio (SDD + TDD)

1. **Spec primero.** No se escribe código ni test nuevo antes de que exista y esté aprobado un
   change en `openspec/changes/<feature>/`. Las 7 capacidades del proyecto ya están **archivadas**
   en `openspec/specs/` (comprobación: `openspec validate --specs --strict`); cualquier trabajo
   nuevo empieza creando un change en `openspec/changes/<feature>/` (`proposal.md`, `design.md`,
   `specs/<capability>/spec.md`, `tasks.md`) con **aprobación explícita del usuario** antes de
   escribir código o tests: ninguna casilla de las fases posteriores de `tasks.md` se marca sin
   esa aprobación sobre proposal + specs + design + tasks.
2. **TDD rojo→verde→refactor**, ejecutando la suite por CLI en cada paso y registrando el rojo
   antes del verde.
3. **Auditoría.** Cada fase la revisa `@rust-reviewer`; sus hallazgos se aplican también con
   TDD (ver las secciones `N.9`/`N.10` de `tasks.md`).
4. **Idioma.** `openspec/config.yaml` fija `Language: es`: los artefactos se escriben en
   español; los encabezados estructurales y `SHALL`/`MUST` se mantienen en inglés.

## Comandos (la CI corre en este orden: fmt → clippy → test)

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test                          # 313 tests, sin red y sin token
cargo test github                   # filtra por nombre (ver convención de tests)
cargo test store                    # persistencia, histórico y agregación por repositorio
cargo test drift                    # clasificación de deriva
cargo test history                  # series y diff
cargo test osv                      # enriquecimiento OSV por lotes
cargo test priority                 # priorización determinista
cargo test report                   # consola, Markdown, JSON y el listado de repositorios
cargo test notify                   # notificador Valet y límites en bytes
cargo test cli                      # subcomandos (incluido `repos`) y códigos de salida
cargo test config                   # precedencia, permisos y valores por defecto
cargo test github::paginacion_sigue # un solo test
cargo test auth::token_redacta
```

- Higiene no negociable: `rustfmt` (`max_width = 100`), Clippy sin warnings
  (`Cargo.toml` fija `[lints.clippy] all = "deny"`) y `unsafe_code = "forbid"`.
- La suite **no usa red ni credenciales**: GitHub se simula con `wiremock`.
- CI: `.github/workflows/ci.yml` ejecuta esos tres pasos en `main` y `development`.

## Convención de tests (no obvia)

- Los tests de `tests/<archivo>.rs` van envueltos en un módulo del mismo nombre
  (`mod github { … }`) para que `cargo test github` los seleccione por nombre.
- Tiempo inyectable, no real: el trait `Sleeper` permite dobles; los tests usan
  `RecordingSleeper`, que registra duraciones sin dormir. Constructor:
  `GithubClient::with_sleeper`.
- Crear tokens en tests con `Token::new("…")` (único constructor; sin `From<String>` público).
- Persistencia: tests con SQLite **en memoria** (`sqlite::memory:`, `max_connections(1)`); las
  migraciones se embeben con `include_str!` y se aplican en runtime (feature `migrate` de
  sqlx), **sin `DATABASE_URL`** ni macros `query!`.

## Invariantes y trampas ya resueltas (no reintroducir)

- **La paginación de Dependabot es por cursor.** El `Link: rel="next"` cambia el prefijo de
  ruta y añade `after=<base64>`; hay que seguir la URL del `Link` **tal cual**, nunca
  recomponerla como `page=N`.
- **`403` no siempre es fallo.** Cuerpo de Dependabot deshabilitado → `GithubError::Disabled`
  (no aborta el run); `403` con `Retry-After` → `RateLimited` (el límite manda).
- **`404` de `code-scanning/alerts` (`no analysis found`) → `GithubError::NotConfigured`**, no
  error.
- Un `3xx` (incluido `304`) es `Http{status}`; no se decodifica su cuerpo.
- Paginación acotada por `GithubConfig::max_pages` + detección de URL repetida →
  `PaginationLimit` (evita colgarse), nunca bucle infinito.
- **El token no se registra nunca.** `Token` redacta `Debug`/`Display`; ningún `AuthError`/
  `GithubError` puede contenerlo; el valor solo sale por `Token::expose_secret()`. Hay tests
  anti-fuga: no los debilites.
- El semáforo de concurrencia solo actúa vía `for_each_concurrent`; si lanzas peticiones por
  tu cuenta, usa `limiter()`.
- **La deriva es un conjunto cerrado** `{new, still_open, resolved, reopened, severity_up}`.
  `resolved_at` es no nulo si y solo si `state = resolved`; `reopened` no es `new`; `severity_up`
  solo sube. El `diff` es **histórico** (lee la instantánea del evento, no el estado actual).
- **No se resuelven alertas de una fuente no observable** (Dependabot `disabled`, code/secret
  scanning sin capacidad): su ausencia es «no se pudo mirar», no «ya no está».
- **`store` depende de `domain`, nunca al revés**; `domain` es puro (sin IO/red/BD).
- **OSV nunca sobreescribe a GitHub.** El enriquecimiento solo completa lo que GitHub no aporta
  (`cvss == 0` cuenta como ausente); los lotes son de ≤1000 consultas y el fallo de OSV **no**
  aborta el escaneo (best-effort con `warn!`). El mapeo de ecosistemas es una tabla cerrada de
  9 entradas; un ecosistema sin equivalencia se omite con `debug!`, sin error.
- **La fecha cruda se normaliza, no se deserializa.** `github::datetime::parse_rfc3339` convierte
  `created_at` (guardado como `Option<String>`) y devuelve `None` si no es RFC 3339 legible: una
  fecha corrupta no debe tumbar la recolección completa del repositorio.
- **La priorización es un orden total y reproducible**: puntúa por severidad, CVSS, versión
  corregida, ámbito (runtime > development) y antigüedad, con desempate hasta el último campo y
  `now` inyectable. No depende del orden de entrada.
- **El informe se escribe SIEMPRE antes de notificar.** `notify::emit_and_notify` escribe Markdown
  y JSON y **después** notifica; un fallo de Valet es best-effort (no altera el exit code ni deja
  el informe a medias). `report` no hace red: la dependencia es unidireccional `notify → report`.
- **El listado de `repos` es un orden total y su filtro no mutila el repositorio.** El orden es
  alertas descendente y, a igualdad, nombre ascendente; `--min-severity` conserva el repositorio
  **entero** si tiene al menos una alerta de esa severidad o superior (no recorta recuento ni
  desglose). `repos_with_alerts`/`total_alerts` son **post-filtro** de `--min-severity` y
  **pre-límite** de `--limit`; `known_repos` es siempre el total de la base. La agregación es una
  consulta `GROUP BY repo` sobre `state = 'open'`, sin cambios de esquema.
- **Los límites de Valet son en bytes, no en caracteres**: `message` ≤4096 B tras `trim` (recorte
  por frontera UTF-8, nunca parte un multibyte) y `metadata` ≤8192 B (reducción que jamás produce
  `400`). Un `401` aborta sin reintentar. El token de ingesta **nunca** sale por log/stdout/JSON.
- **La config de Valet exige permisos `600`**: precedencia **entorno > fichero**
  (`~/.config/ghwatch/config.toml`); un fichero más laxo se rechaza con aviso y **no se usa**.
- **Un test de logs con `tracing` no es fiable sin `rebuild_interest_cache()`** y sin serializar los
  tests que comparten callsite: el interés se cachea globalmente y el evento puede no emitirse.

## Credenciales y entorno

- Resolución del token, en orden: `GITHUB_TOKEN` → `gh auth token` → error explicativo.
  Scope clásico necesario: `repo`.
- Git ignora y no versionar: `*.db`/`*.sqlite*`, `.env*`, `/config.toml` (la config de
  ejecución vive en `~/.config/ghwatch/config.toml`).
- Contrato con Valet: `POST {VALET_BASE_URL}/api/notifications` con ingest key Bearer;
  spec en `/data/rust/valet/docs/api-notifications.md` y en `PLAN.md` §5.

## Git Flow

Este proyecto sigue Git Flow estricto. Consulta [`GIT_FLOW.md`](GIT_FLOW.md) para:

- La estructura de ramas (`main`, `development`, `feature/*`, `hotfix/*`, `release/*`).
- Los *conventional commits* con gitmoji y la tabla de bumps.
- Cómo crear features, hotfixes y releases (bump con `vampus`, changelog con `git-cliff`,
  tag en `main` que dispara `release.yml` y sincronización de `development`).

## Ramas y commits

- Gitflow con gitmoji: `main` y `development` **solo** por PR; trabajo diario en `feature/*`
  (patrón actual: `feature/ghwatch-<fase>-<nombre>`); commits `<emoji> <tipo>(<scope>): <msg>`.
- El stack, el modelo de datos SQLite (4 tablas) y el mapeo de ecosistemas Dependabot→OSV
  están en `PLAN.md`. No inventes esquemas: consúltalos.
