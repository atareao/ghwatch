# Design: cli-repos-list

## Context

Ver `proposal.md` — Why. Estado actual que condiciona el diseño:

- `src/cli.rs` cablea seis subcomandos y despacha en `run_with`; `history` y `diff` ya leen **solo** de la base de datos a través de `store.query`.
- `store::Query` expone `alerts()`, `alerts_for_repo()`, `runs()`, `events_for_run()`, `history()`, `diff()` y `latest_run_id()`, pero **ninguna** agregación por repositorio.
- `Severity` (`src/domain/alert.rs`) ya aporta `rank()` (orden relativo, con `Unknown` = 0) y `as_str()` (texto estable): bastan para el filtro y el desglose.
- `Env` (`src/cli.rs`) inyecta `now`, `out`, el acceso al entorno y el resolutor de credenciales: los tests no tocan el proceso ni la red.
- `report` **no** consulta la base de datos: reemite el último informe guardado en `--report-dir`. Por eso el listado completo no se puede obtener con lo existente.

## Goals / Non-Goals

**Goals:**

- Obtener el listado completo de repositorios con sus alertas desde el CLI, sin abrir SQLite a mano.
- Una sola consulta agregada, de solo lectura, determinista y reproducible.
- Reutilizar las convenciones ya vigentes: precedencia de configuración, códigos de salida, reloj y salida inyectables.

**Non-Goals:**

- No se toca `report` ni el informe: la tabla de exactamente 15 repositorios sigue siendo la del informe (spec `reporting`).
- No se escribe ningún fichero: `repos` imprime en la salida estándar, como `history` y `diff`. Un `--output` queda para otro change.
- No se listan repositorios sin alertas abiertas (`--include-empty` queda fuera).
- No se filtra por fuente (`--source`) ni por estado (`--state`): se cuentan las alertas **abiertas**.

## Decisions

### Un subcomando nuevo `repos`, no una opción de `report`

`report` reemite el informe guardado y su contrato (spec `reporting`) fija una tabla de exactamente 15 repositorios. Añadir `--all` a `report` cambiaría ese contrato y mezclaría dos responsabilidades: reemitir un documento frente a consultar la base de datos. `repos` se suma a la familia de consultas (`diff`, `history`) que ya leen de SQLite.

**Alternativas descartadas:** (a) `report --all-repos` — rompe el contrato de `reporting`; (b) `history --repos` — mezcla dos ejes (ejecuciones frente a repositorios).

### Agregación en SQL, no en memoria

La consulta se resuelve con un `GROUP BY repo` sobre `alerts` filtrando `state = 'open'`, no cargando todas las alertas en memoria como hace `alerts()`. El caso real son 1.330 filas, pero la agregación escala sin cambiar el coste de la consulta y evita materializar el histórico completo.

**Alternativa descartada:** reutilizar `alerts()` y agregar en Rust — más simple hoy, pero carga todas las alertas en memoria y desaprovecha los índices de SQLite.

### Orden total y reproducible

El listado se ordena por número de alertas **descendente** y, a igualdad, por nombre **ascendente**. Es un orden total: dos ejecuciones sobre el mismo estado producen exactamente la misma salida, lo que hace el test determinista sin depender del orden que devuelva SQLite.

### El filtro de severidad actúa sobre el repositorio, no sobre las alertas

`--min-severity` conserva un repositorio si tiene **al menos una** alerta abierta de esa severidad o superior; el recuento y el desglose que se muestran siguen siendo **todas** sus alertas abiertas. Es la lectura útil de la pregunta «¿qué repositorios tienen críticas?»: el repositorio aparece entero, no mutilado.

### El filtro se parsea en el CLI; `domain` sigue siendo puro

`--min-severity` se declara como un `ValueEnum` propio de `src/cli.rs` (`MinSeverity`) que se traduce a `Severity` mediante `rank()`. `Severity` **no** deriva `clap::ValueEnum`: `domain` es puro y no debe conocer el parseo de la línea de comandos, y `store` depende de `domain` (nunca al revés). Un valor fuera del conjunto es error de uso (código 2) en lugar de degradar en silencio a `Unknown`.

**Alternativa descartada:** derivar `ValueEnum` sobre `Severity` — mete `clap` en el modelo de dominio; y un `String` libre con `Severity::from_github` convertiría un valor inválido en `Unknown` y **desactivaría el filtro sin avisar**.

### `--limit` con 0 = todos, y `--all` manda

`--limit <N>` con defecto 20 (coherente con `history`); `--limit 0` y `--all` muestran todos. `--all` es azúcar de `--limit 0` y prevalece si se combinan.

### Sin cambios de esquema ni de dependencias

La consulta lee `alerts` y `repos`, ya existentes: no hay migración. No se añade ninguna dependencia: el JSON se serializa con `serde_json` y el Markdown se compone a mano, como en el resto de `src/report/`. `generated_at` sale de `Env::now`, inyectable, de modo que el documento es determinista en los tests.

## Risks / Trade-offs

- **[La salida JSON estrena un documento con su propio `schema_version`]** → se versiona igual que el informe y su forma queda documentada en la spec `reporting`, para poder evolucionarla sin romper consumidores.
- **[`--min-severity unknown` deja pasar todos los repositorios con alertas]** → es coherente con `rank()` (`Unknown` = 0); queda documentado en `--help` y en la spec.
- **[El listado no distingue la fuente]** → las alertas de secret scanning se cuentan igual que las de Dependabot; un desglose por fuente queda para otro change si hiciera falta.

## Migration Plan

No hay migración: subcomando aditivo, sin cambios de esquema ni de formatos existentes. Reversión = revertir el commit; no deja estado.

## Open Questions

Ninguna: las decisiones anteriores cierran el diseño. Si más adelante se quiere el listado en fichero (`--output`), se abre otro change.
