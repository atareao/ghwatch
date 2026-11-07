# PLAN-001 — `ghwatch repos`: extensiones (salida a fichero, repositorios sin alertas, fuente y estado)

> Plan de proyecto del change OpenSpec **`cli-repos-extended`**
> (`openspec/changes/cli-repos-extended/`), que es el contrato normativo.
> Este fichero es el índice de trabajo; ante discrepancia, mandan los deltas de spec.

## Objetivo

Cerrar los cuatro follow-ups documentados en `docs/cli-repos-list.md`:

1. `--output <ruta>` — volcar el listado a un fichero.
2. `--include-empty` — incluir los repositorios del inventario sin alertas.
3. `--source <dependabot|secret|code>` — ceñir el listado a una fuente (filtro).
4. `--state <open|resolved|all>` — consultar abiertas, resueltas o ambas.

## Secciones

| # | Contenido | Ficheros | Auditoría |
|---|---|---|---|
| 1 | `RepoQuery` + `LEFT JOIN` con estado, fuente e `include_empty`; `repos_with_alerts` | `src/store/query.rs`, `tests/store.rs` | `@rust-reviewer` |
| 2 | Documento del listado esquema 2 y las tres presentaciones | `src/report/repos.rs`, `tests/report.rs` | `@rust-reviewer` |
| 3 | `--output`, `--include-empty`, `--source`, `--state` en el subcomando | `src/cli.rs`, `tests/cli.rs` | `@rust-reviewer` |
| 4 | Documentación | `AGENTS.md`, `PLAN.md`, `README.md`, `README.es.md`, `docs/cli-repos-list.md` | — |
| 5 | Verificación final contra la base real, puerta de calidad y archivado | — | — |

## Puerta de calidad

`cargo fmt --all --check` · `cargo clippy --all-targets -- -D warnings` · `cargo test`
(314 tests previos, todos verdes).

## Cierre

Al terminar: marcar las casillas de `tasks.md`, archivar el change
(`openspec archive cli-repos-extended --yes`) y renombrar este plan a
`plans/PLAN-001-ARCHIVED.md`.
