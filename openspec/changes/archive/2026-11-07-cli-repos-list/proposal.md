# Proposal: cli-repos-list

## Why

El listado de repositorios con su número de alertas solo existe **capado a 15**: la tabla «Repositorios con más alertas» del informe Markdown y el array `worst_repos` del JSON. Para saber qué repositorios tienen vulnerabilidades —y cuántas— hay que ir a SQLite a mano, porque `ghwatch report` ni siquiera consulta la base de datos: reemite el último informe guardado. Con 88 repositorios afectados sobre 363 escaneados, el top 15 deja fuera a la mayoría.

## What Changes

- Nuevo subcomando `ghwatch repos` que lista, **desde la base de datos local y sin red**, cada repositorio con al menos una alerta abierta, su número de alertas abiertas y su desglose por severidad.
- Acotación de la salida: `--limit <N>` (20 por defecto), y `--limit 0` / `--all` para mostrarlos todos.
- Filtrado: `--min-severity <critical|high|medium|low|unknown>`, que conserva solo los repositorios con al menos una alerta abierta de esa severidad o superior.
- Formatos: `--format console|json|markdown`, con el mismo contenido en los tres, escritos en la salida estándar.
- Ubicaciones: `--db` y `--config`, con la precedencia ya vigente en el resto del CLI.
- Documentación del subcomando en la ayuda, `AGENTS.md`, `PLAN.md` y los dos README.

Sin **BREAKING**: `repos` es aditivo; ningún subcomando ni formato existente cambia de comportamiento.

## Capabilities

### New Capabilities

Ninguna.

### Modified Capabilities

- `cli`: el binario pasa a exponer **siete** subcomandos (`repos` se suma a `scan`, `report`, `diff`, `history`, `notify` e `init`); `repos` se incorpora a los subcomandos de consulta que solo leen del almacenamiento local; y se define su contrato de opciones, orden, formatos y códigos de salida.
- `reporting`: se añade el contrato de presentación del listado de repositorios en consola, Markdown y JSON.

## Impact

- Código: `src/cli.rs` (variante y argumentos nuevos, más `ejecutar_repos`), `src/store/query.rs` (agregación por repositorio) y un módulo de presentación nuevo bajo `src/report/`.
- Sin cambios de esquema: la consulta lee las tablas `alerts` y `repos` ya existentes; no hay migración.
- Sin dependencias nuevas, sin red y sin credenciales.
- Documentación: `AGENTS.md`, `PLAN.md`, `README.md`, `README.es.md` y `docs/cli-repos-list.md`.
- Tests: integración de CLI (nueva salida y códigos de salida), unitarios de la agregación y de los tres formatos.
