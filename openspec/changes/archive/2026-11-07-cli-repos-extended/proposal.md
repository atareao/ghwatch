# Proposal: cli-repos-extended

## Why

`ghwatch repos` (change archivado `cli-repos-list`) resuelve la consulta básica: lista los repositorios con alertas **abiertas**, acotados con `--limit`/`--all` y filtrados con `--min-severity`, en consola, Markdown o JSON por la salida estándar. `docs/cli-repos-list.md` deja constancia de cuatro huecos que impiden los casos de uso siguientes:

1. **Volcar el listado a fichero.** Hoy solo hay salida estándar; archivar el listado o consumirlo desde otro proceso exige redirecciones de shell.
2. **Ver los repositorios sin alertas.** El listado solo muestra los 88 repositorios con alertas de los 363 del inventario: los 275 restantes —y su ausencia de alertas— son invisibles.
3. **Ceñir el listado a una fuente.** Dependabot, secret scanning y code scanning se cuentan juntos; no hay forma de ver solo una fuente.
4. **Consultar alertas resueltas.** El listado fija `state = 'open'`: las alertas resueltas no se pueden consultar aunque estén en la base de datos.

## What Changes

- **`--output <ruta>`**: escribe la salida formateada en el fichero indicado —lo crea o lo sobrescribe— en lugar de la salida estándar. Vale para los tres formatos.
- **`--include-empty`**: incluye también los repositorios del inventario **sin** ninguna alerta, con recuento y desglose a cero. `known_repos` sigue siendo el total de la base de datos.
- **`--source <dependabot|secret|code>`**: cuenta solo las alertas de esa fuente. Es un **filtro**, no un desglose por fuente en la salida.
- **`--state <open|resolved|all>`**: estado de las alertas contadas, con `open` por defecto (el comportamiento actual). `all` cuenta abiertas y resueltas juntas.
- **Documento del listado (esquema 2)**: se añaden los campos que describen el conjunto —`state`, `source`, `min_severity`, `include_empty` y `listed_repos`— para que el JSON sea autoexplicativo, y se ajustan las tres presentaciones (consola, Markdown y JSON) a los filtros.

Sin **BREAKING** de facto: sin las opciones nuevas, `repos` se comporta como hasta ahora (mismos números; cambia la redacción de la cabecera para nombrar el estado). El JSON sube de `schema_version` 1 a 2, que es el mecanismo previsto para crecer.

## Capabilities

### New Capabilities

Ninguna.

### Modified Capabilities

- `cli`: el contrato de `ghwatch repos` crece con `--output`, `--include-empty`, `--source` y `--state`, y se generaliza el requirement del listado (ya no habla solo de alertas «abiertas»).
- `reporting`: el documento del listado declara el esquema 2 (filtros y `listed_repos`) y las tres presentaciones reflejan el estado consultado.

## Impact

- Código: `src/store/query.rs` (la consulta pasa a `LEFT JOIN` sobre `repos` con filtros opcionales de estado y fuente), `src/report/repos.rs` (documento v2 y las tres salidas) y `src/cli.rs` (`--output` y los tres filtros nuevos, más la escritura del fichero).
- **Sin cambios de esquema**: la consulta lee las tablas `alerts` y `repos` ya existentes (el índice `idx_alerts_repo` ya cubre el `JOIN`); no hay migración.
- Sin dependencias nuevas, sin red y sin credenciales.
- Documentación: `AGENTS.md`, `PLAN.md`, `README.md`, `README.es.md` y `docs/cli-repos-list.md`.
- Tests: unitarios de la consulta (estado, fuente, `include_empty`), del documento v2 y de las tres salidas, e integración de CLI (`--output`, filtros y códigos de salida).
