# Tasks: cli-repos-extended

## 0. Aprobación (punto de parada del protocolo)

- [x] 0.1 Presentar este change (proposal + specs + design + tasks) y esperar aprobación explícita antes de tocar código.

## 1. Consulta agregada: estado, fuente y repositorios sin alertas

- [x] 1.1 RED — escribir en `tests/store.rs` (dentro de `mod store`) los tests de `RepoQuery`: estado (`open`/`resolved`/`all`), fuente (`dependabot`/`secret`/`code`) e `include_empty`, sobre una base en memoria; comando `cargo test store`; pegar la salida en rojo.
- [x] 1.2 RED — escribir el test del repositorio sin alertas: con `include_empty` aparece con `alerts = 0` y los cinco cubos a cero (guardianes `COUNT(a.id)`, `a.id IS NOT NULL` y `COALESCE(MAX(...), 0)`); sin `include_empty` no aparece; comando `cargo test store`; pegar la salida en rojo.
- [x] 1.3 RED — escribir el test de `repos_with_alerts` (cuenta solo los repositorios con al menos una alerta) y del orden con repositorios vacíos (van al final, por nombre ascendente); comando `cargo test store`; pegar la salida en rojo.
- [x] 1.4 GREEN — implementar en `src/store/query.rs` el tipo `RepoQuery` y la consulta con `LEFT JOIN` sobre `repos`, los tres guardianes y el `HAVING` acumulativo; añadir `repos_with_alerts` a `RepoAlertList`; confirmar con `cargo test store` y `cargo check`.
- [x] 1.5 REFACTOR — doc-comments (la trampa del `LEFT JOIN` y la semántica de `repos_with_alerts`); confirmar `cargo clippy --all-targets -- -D warnings` y `cargo fmt --all --check`.
- [x] 1.6 Auditoría con `@rust-reviewer` de la consulta y de su cobertura.

### 1.7 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la sección 1 (veredicto: aprobado con reservas).

- [x] 1.7.1 Hallazgo 1 — el doc-comment de `repo_alert_counts` documenta la precondición del `LEFT JOIN` (alertas huérfanas). Verificación: `cargo test store`.
- [x] 1.7.2 Hallazgo 3 — se elimina el sombreo del helper `abiertas` en `tests/store.rs`. Verificación: `cargo test store`.

## 2. Documento del listado (esquema 2) y las tres presentaciones

- [x] 2.1 RED — escribir en `tests/report.rs` (dentro de `mod report`) los tests del documento v2: `state`, `source`, `min_severity`, `include_empty`, `listed_repos` y `repos_with_alerts`; comando `cargo test report`; pegar la salida en rojo.
- [x] 2.2 RED — escribir los tests de la cabecera de consola según el estado (abiertas/resueltas/todas), del listado vacío (`sin repositorios que mostrar`) y de la tabla Markdown con el estado consultado; comando `cargo test report`; pegar la salida en rojo.
- [x] 2.3 GREEN — implementar el enum `RepoState`, los campos nuevos de `ReposDocument` (subiendo `REPOS_SCHEMA_VERSION` a 2) y las tres presentaciones; confirmar con `cargo test report`.
- [x] 2.4 REFACTOR — doc-comments (semántica de los cuatro totales) y puerta de calidad del módulo.
- [x] 2.5 Auditoría con `@rust-reviewer` del documento y de las tres presentaciones.

### 2.6 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la sección 2 (veredicto: aprobado con reservas).

- [x] 2.6.1 Hallazgo 1 — el delta de `reporting` acota «los filtros aplicados» al documento JSON (el estado sí va en los tres formatos), en coherencia con la decisión 5 del diseño. Verificación: `openspec validate cli-repos-extended --strict`.
- [x] 2.6.2 Hallazgo 3 — test del orden de las claves del contrato JSON. Verificación: `cargo test report`.
- [x] 2.6.3 Hallazgo 4 — el assert del título Markdown en el CLI se estrecha a `# Repositorios con alertas abiertas`. Verificación: `cargo test cli`.

## 3. Subcomando: `--output`, `--include-empty`, `--source` y `--state`

- [ ] 3.1 RED — escribir en `tests/cli.rs` (dentro de `mod cli`) los tests de `--include-empty`, `--source` y `--state` sobre una base sembrada (`Store::connect` + `persist_scan`); comando `cargo test cli`; pegar la salida en rojo.
- [ ] 3.2 RED — escribir los tests de `--output`: el fichero contiene el listado, la salida estándar queda vacía y el código es 0; y el fallo de escritura (carpeta inexistente) devuelve 5; comando `cargo test cli`; pegar la salida en rojo.
- [ ] 3.3 RED — escribir los tests de los valores inválidos (`--source github`, `--state volar` → error de uso, código 2) y de la ayuda de `repos` (nombra las opciones nuevas); comando `cargo test cli`; pegar la salida en rojo.
- [ ] 3.4 GREEN — añadir `SourceFilter` y `StateFilter` (`ValueEnum`), las cuatro opciones de `ReposArgs` y la escritura del fichero en `ejecutar_repos`; confirmar con `cargo test cli`.
- [ ] 3.5 REFACTOR — confirmar la puerta de calidad completa: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test` (los 314 tests previos siguen verdes).
- [ ] 3.6 Auditoría con `@rust-reviewer` del subcomando, de los códigos de salida y de la escritura del fichero.

## 4. Documentación

- [ ] 4.1 Actualizar `AGENTS.md` (las cuatro opciones nuevas y el recuento de tests) y verificar que la lista de subcomandos coincide con `ghwatch --help`.
- [ ] 4.2 Actualizar `PLAN.md` con las opciones nuevas y verificar que no contradice la sección de modelo de datos.
- [ ] 4.3 Actualizar `README.md` y `README.es.md` con las cuatro opciones y un ejemplo; verificar que el ejemplo, ejecutado contra la base real, produce esa forma.
- [ ] 4.4 Actualizar `docs/cli-repos-list.md` (funcionamiento, configuración, uso, decisiones y follow-ups) y verificar que todos los comandos que cita funcionan tal cual.

## 5. Verificación final

- [ ] 5.1 Ejecutar `ghwatch repos --all` y `ghwatch repos --include-empty --all` contra la base de datos real y comprobar que los totales coinciden con `ghwatch report`; pegar la salida.
- [ ] 5.2 Ejecutar la puerta de calidad completa (`cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) y pegar el resumen.
- [ ] 5.3 Verificar requirement por requirement del change contra la implementación y marcar las casillas de las secciones 1 a 4.
