# AGENTS.md — ghwatch

CLI en Rust (edición 2024) que vigilará vulnerabilidades en todos los repos GitHub de una
cuenta (Dependabot + secret/code scanning, enriquecido con OSV.dev, con histórico SQLite e
informes). Especificación viva: `openspec/`. Checkpoint arquitectónico: `PLAN.md`.

## Estado actual — leer antes de tocar código

- **`src/main.rs` es un stub que imprime `Hello, world!`.** No hay CLI (ni `clap`) ni
  `config.rs`. `cargo install ghwatch` **no** da un escáner.
- Implementado y auditado (biblioteca): `auth`; `github` (cliente HTTP + recolección del
  inventario y de las tres fuentes de alertas); `domain` (tipos puros); `scan` (orquestación
  con concurrencia acotada); `store` (persistencia SQLite, ciclo de vida y deriva).
- **Sin escribir todavía** (planificado en `PLAN.md`): `cli`, `config`, `osv`, `report`,
  `notify`, y el cableado de los subcomandos `scan|report|diff|history|notify|init`.
- No implementes módulos ni "arregles" el stub por iniciativa propia: el trabajo pasa por el
  flujo de spec (abajo).

## Flujo obligatorio (SDD + TDD)

1. **Spec primero.** No se escribe código ni test nuevo antes de que exista y esté aprobado un
   change en `openspec/changes/<feature>/`. El change activo es `ghwatch-scanner`
   (`proposal.md`, `design.md`, `specs/<capability>/spec.md`, `tasks.md`).
   `tasks.md` §0 es un **punto de parada**: ninguna casilla de las fases posteriores se marca
   sin aprobación explícita del usuario sobre proposal + specs + design + tasks.
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
cargo test                          # 151 tests, sin red y sin token
cargo test github                   # filtra por nombre (ver convención de tests)
cargo test store                    # persistencia e histórico
cargo test drift                    # clasificación de deriva
cargo test history                  # series y diff
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

## Credenciales y entorno

- Resolución del token, en orden: `GITHUB_TOKEN` → `gh auth token` → error explicativo.
  Scope clásico necesario: `repo`.
- Git ignora y no versionar: `*.db`/`*.sqlite*`, `.env*`, `/config.toml` (la config de
  ejecución vive en `~/.config/ghwatch/config.toml`).
- Contrato con Valet: `POST {VALET_BASE_URL}/api/notifications` con ingest key Bearer;
  spec en `/data/rust/valet/docs/api-notifications.md` y en `PLAN.md` §5.

## Ramas y commits

- Gitflow con gitmoji: `main` y `development` **solo** por PR; trabajo diario en `feature/*`
  (patrón actual: `feature/ghwatch-<fase>-<nombre>`); commits `<emoji> <tipo>(<scope>): <msg>`.
- El stack, el modelo de datos SQLite (4 tablas) y el mapeo de ecosistemas Dependabot→OSV
  están en `PLAN.md`. No inventes esquemas: consúltalos.
