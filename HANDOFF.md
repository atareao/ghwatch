# HANDOFF — ghwatch

## Estado

- `main` = `development` = `602721c` (**v0.2.0**). Repositorio sincronizado y árbol limpio.
- Los tres pendientes del plan están **cerrados**:
  1. **Release v0.2.0** publicada: tag `v0.2.0` en `main`, `release.yml` verde, dos
     artefactos (`ghwatch-x86_64-unknown-linux-gnu.tar.gz`,
     `ghwatch-aarch64-unknown-linux-gnu.tar.gz`). Binario local recompilado con
     `just install` → `~/.local/bin/ghwatch` ya es **0.2.0** (con `repos`).
  2. **CI en Node.js 24**: `checkout@v7`, `upload-artifact@v7`,
     `download-artifact@v8`, `action-gh-release@v3` (PR #13). Validado de extremo a
     extremo por el propio `release.yml` del tag.
  3. **`ghwatch repos` extendido** (change `cli-repos-extended`, PR #12): `--output`,
     `--include-empty`, `--source` y `--state`. Archivado en
     `openspec/changes/archive/2026-11-07-cli-repos-extended/`.
- Contenido de la v0.2.0: subcomando `ghwatch repos` (`cli-repos-list` + extensiones)
  y la subida de acciones de CI. Suite: **334 tests** verdes; `fmt` y
  `clippy -D warnings` limpios; `openspec validate --specs --strict` → 7/7.
- Contra la base real (`~/.local/share/ghwatch/ghwatch.db`, ejecución #3):
  `repos --all` → 88 repositorios con alertas de 363, 1330 alertas abiertas
  (33/517/573/206/1), idéntico a `report --format json`.

## Pendiente

- **Nada abierto** de los tres items. Sin PRs abiertos; solo ramas `main` y
  `development` (local y remota).
- **Deuda consciente de `repos`** (documentada en `docs/cli-repos-list.md`):
  acoplamiento del SQL al conjunto de severidades, dos lecturas no atómicas,
  `--output` sin crear directorios y el desglose por fuente (`by_source`) fuera
  de alcance.
- **Aviso de runner**: GitHub avisa de que `ubuntu-latest` migrará a Ubuntu 26 a
  partir del 2026-10-19; revisar `release.yml`/`ci.yml` cuando ocurra.

## Notas

- El informe (`report`) sigue exponiendo solo el **top 15** (`worst_repos` y la
  tabla Markdown); su contrato no se ha tocado.
- Flujo SDD + TDD por secciones: cada sección con rojo→verde, auditoría de
  `@rust-reviewer` y endurecimiento (ver los `tasks.md` archivados).
