# HANDOFF — ghwatch

## Estado

- `main` = `d0f4afe` (v0.1.0). `development` = `62a96df` (Merge PR #11).
- Rama activa: **`feature/cli-repos-extended`**; PR **#12** abierto (borrador) a
  `development`, pendiente de merge.
- Feature **`cli-repos-extended` completada, verificada y archivada**: `ghwatch
  repos` gana cuatro opciones y cierra los follow-ups de `cli-repos-list`.
  - `--output <ruta>`: vuelca la salida a fichero (mismos bytes que stdout) y
    calla stdout; fallo de escritura → código 5; no crea directorios.
  - `--include-empty`: añade los repositorios del inventario sin alertas (0/0/0),
    al final del orden.
  - `--source <dependabot|secret|code>`: filtro por fuente (no desglose).
  - `--state <open|resolved|all>`: estado contado, `open` por defecto.
  - La consulta pasa a `LEFT JOIN` `repos`→`alerts` con `RepoQuery`; el documento
    del listado sube a `schema_version` 2 (`state`, `source`, `min_severity`,
    `include_empty`, `listed_repos`).
  - Change archivado en `openspec/changes/archive/2026-11-07-cli-repos-extended/`;
    deltas fusionados en `openspec/specs/cli/spec.md` (3 modified + 3 renamed) y
    `openspec/specs/reporting/spec.md` (1 modified).
  - `openspec validate --specs --strict` → 7/7 capacidades válidas.
  - Suite: **334 tests** verdes; `cargo fmt --all --check` y
    `cargo clippy --all-targets -- -D warnings` limpios.
  - Documentación: `AGENTS.md`, `PLAN.md`, `README.md`, `README.es.md` y
    `docs/cli-repos-list.md`; plan de proyecto `plans/PLAN-001-ARCHIVED.md`.
- Contra la base real (`~/.local/share/ghwatch/ghwatch.db`, ejecución #3):
  `ghwatch repos --all` → 88 repositorios con alertas de 363, 1330 alertas
  abiertas (33 críticas / 517 altas / 573 medias / 206 bajas / 1 desconocida),
  y coincide con `ghwatch report --format json`. `--include-empty --all` → 363 de
  363 con 88 con alertas; `--state resolved --all` → 2 repos, 8 resueltas.

## Pendiente

- **Merge del PR #12** (`feature/cli-repos-extended` → `development`): quitar el
  borrador y fusionar; borrar la rama.
- **Deprecación de Node.js 20** en los workflows (item 2): `actions/checkout@v4`,
  `upload-artifact@v4`, `download-artifact@v4`, `softprops/action-gh-release@v2`.
- **Release v0.2.0** (bump *minor*, item 1): rama `release/v0.2.0` → PR a `main`,
  tag `v0.2.0` que dispara `release.yml`, y sincronizar `development`. El binario
  instalado en `~/.local/bin/ghwatch` es aún la v0.1.0: recompilar con
  `just install` tras el release.
- **Deuda consciente de `repos`** (documentada en `docs/cli-repos-list.md`):
  acoplamiento del SQL al conjunto de severidades, dos lecturas no atómicas,
  `--output` sin crear directorios y el desglose por fuente (`by_source`) fuera
  de alcance.

## Notas

- La prioridad para el usuario era poder consultar el listado completo de
  repositorios con su número de vulnerabilidades **desde el propio `ghwatch`**,
  sin abrir SQLite a mano: resuelto con `ghwatch repos --all`.
- El informe (`report`) sigue exponiendo solo el **top 15** (`worst_repos` y la
  tabla Markdown); no se ha tocado su contrato.
