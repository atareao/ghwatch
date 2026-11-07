# Feature: `cli-repos-list` — listado de repositorios con alertas

> Un fichero por feature. Esta es la documentación de la feature `cli-repos-list`
> (subcomando `ghwatch repos`). Spec viva en `openspec/changes/cli-repos-list/`
> (a archivar en `openspec/specs/` al cerrar el change).

## Qué es

`ghwatch repos` es un **subcomando de solo lectura** que lista, **desde la base de
datos local y sin red**, cada repositorio con al menos una alerta **abierta**, su
número de alertas abiertas y su desglose por severidad.

Nace de una limitación concreta: el listado de repositorios solo existía **capado
a 15** (la tabla «Repositorios con más alertas» del informe Markdown y el array
`worst_repos` del JSON), y `ghwatch report` ni siquiera consulta la base de datos
(reemite el último informe guardado). Con 88 repositorios afectados sobre 363
escaneados, el top 15 dejaba fuera a la mayoría; para ver el resto había que abrir
SQLite a mano. `repos` cubre ese hueco.

Características:

- **Sin red, sin credenciales y sin escrituras**: lee las tablas `alerts` y `repos`
  ya existentes; no crea la base de datos si no existe.
- Orden **total y reproducible**: alertas descendente y, a igualdad, nombre
  ascendente.
- Acotación (`--limit`/`--all`) y filtro por severidad mínima (`--min-severity`).
- Tres formatos en **stdout**: `console` (defecto), `json` y `markdown`.
- **Sin cambios de esquema** y **sin dependencias nuevas**.

## Ficheros implicados

| Fichero | Papel |
|---|---|
| `src/store/query.rs` | Agregación `GROUP BY repo`: `Store::repo_alert_counts`, tipos `RepoAlertCount` y `RepoAlertList`. |
| `src/report/repos.rs` | Presentación pura (nuevo): `ReposDocument`, `RepoEntry`, `RepoSeverityCounts`, `REPOS_SCHEMA_VERSION`. |
| `src/report/mod.rs` | Reexporta el módulo `repos`. |
| `src/cli.rs` | `Command::Repos`, `ReposArgs`, `MinSeverity` y `ejecutar_repos`. |
| `tests/store.rs` | Tests de la agregación (dentro de `mod store`). |
| `tests/report.rs` | Tests de los tres formatos (dentro de `mod report`). |
| `tests/cli.rs` | Tests del subcomando, opciones y códigos de salida (dentro de `mod cli`). |

Sin migraciones: `migrations/` no se toca.

## Cómo funciona

### Consulta agregada

`Store::repo_alert_counts(min_severity)` resuelve **una sola consulta** agregada:

```sql
SELECT repo, COUNT(*) AS alerts,
       SUM(CASE WHEN severity = 'critical' THEN 1 ELSE 0 END),
       SUM(CASE WHEN severity = 'high'     THEN 1 ELSE 0 END),
       SUM(CASE WHEN severity = 'medium'   THEN 1 ELSE 0 END),
       SUM(CASE WHEN severity = 'low'      THEN 1 ELSE 0 END),
       SUM(CASE WHEN severity IS NULL
         OR severity NOT IN ('critical','high','medium','low') THEN 1 ELSE 0 END)
FROM alerts WHERE state = 'open'
GROUP BY repo
HAVING MAX(CASE severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3
  WHEN 'medium' THEN 2 WHEN 'low' THEN 1 ELSE 0 END) >= ?1
ORDER BY COUNT(*) DESC, repo ASC
```

- **Solo se cuentan las alertas abiertas** (`state = 'open'`); las resueltas no
  entran ni en el recuento ni en el desglose.
- El bucket `unknown` recoge cualquier severidad fuera del conjunto conocido (y
  `NULL`): así se mantiene el invariante
  `critical + high + medium + low + unknown == alerts`.
- La agregación se hace **en SQL**, no cargando todas las alertas en memoria.
- El `COUNT(*)` de repositorios conocidos (`known_repos`) es una **segunda**
  lectura sobre `repos` (ver «Decisiones» y «Follow-ups»).

### Orden

Orden **total**: `alertas DESC, repo ASC`. No depende del orden de inserción ni
del plan de ejecución de SQLite: el mismo estado produce la misma salida.

### Filtros

- `--min-severity <critical|high|medium|low|unknown>` conserva un repositorio si
  tiene **al menos una** alerta de esa severidad **o superior**, **sin recortar**
  el recuento ni el desglose mostrados: el repositorio aparece **entero**.
  `--min-severity unknown` (rank 0) deja pasar todos los repositorios con alertas.
- Un valor fuera del conjunto (`--min-severity catastrofica`) es **error de uso**
  (código 2), no un `Unknown` silencioso.
- `--limit <N>` acota cuántas filas se muestran; **defecto 20**. `--limit 0` y
  `--all` muestran todos, y `--all` **prevalece** si se combinan.

### Formatos

Los tres formatos llevan la **misma información** y se escriben **siempre a
stdout** (no se crea ni modifica ningún fichero).

- **`console`** (defecto): una línea de cabecera y una línea por repositorio:

  ```
  ejecución #{run_id} · generado {rfc3339} · repositorios con alertas: {n} de {total} · alertas abiertas: {total_alertas}
  {repo}  {alertas}  (critical=.. high=.. medium=.. low=.. unknown=..)
  ```

  Con el listado vacío, tras la cabecera: `sin repositorios con alertas`.

- **`markdown`**: cabecera `# Repositorios con alertas`, viñetas con ejecución,
  generado, repositorios y alertas, y una tabla
  `| Repositorio | Alertas | Críticas | Altas | Medias | Bajas | Desconocidas |`.
  Con el listado vacío, una fila `| (ninguno) | 0 | 0 | 0 | 0 | 0 | 0 |`.

- **`json`**: `to_string_pretty` con las claves estables `schema_version`,
  `generated_at`, `run_id`, `known_repos`, `repos_with_alerts`, `total_alerts` y
  `repos[]` (cada uno con `repo`, `alerts` y `by_severity`
  `{critical, high, medium, low, unknown}`).

### Semántica de los totales

| Campo | Significado |
|---|---|
| `known_repos` | Repositorios de la tabla `repos` (total de la base, **al margen del filtro**). |
| `repos_with_alerts` | Repositorios del conjunto: **post-filtro** de `--min-severity` y **pre-límite** de `--limit`. |
| `total_alerts` | Suma de alertas del conjunto: **post-filtro** y **pre-límite**. |

Es decir, `repos_with_alerts` y `total_alerts` describen **lo que devolvió el
almacén**, no la parte que se muestra tras recortar con `--limit`.

## Variables y configuración

`repos` **no tiene variables de entorno propias**: reutiliza la resolución de
configuración común al resto del CLI.

- `--db <ruta>`: ruta de la base de datos. Se resuelve con la precedencia general:
  **flags > entorno (`GHWATCH_DB`) > fichero (`db_path` en `config.toml`) >
  defecto** (`~/.local/share/ghwatch/ghwatch.db`).
- `--config <ruta>`: ruta del fichero de configuración. Por defecto,
  `~/.config/ghwatch/config.toml` (el fichero debe conservar permisos `600`). Si
  la ruta indicada no existe, se avisa y se usa la ruta por defecto.

A diferencia de `scan`, `repos` **no resuelve credenciales** y **no contacta con
la red**. Si la base de datos no existe, **no la crea**: avisa
`sin datos locales (ejecuta \`ghwatch scan\`)` y termina con `0`.

## Cómo usarlo

Todos los ejemplos siguientes funcionan tal cual. Los que dicen «requiere la base
real» necesitan una base con datos (por ejemplo, la que deja `ghwatch scan`); los
demás no necesitan ninguna.

```bash
# Los cinco repositorios con más alertas abiertas (requiere la base real)
ghwatch repos --limit 5
```

```
ejecución #3 · generado 2026-11-07T08:25:50.008680277+00:00 · repositorios con alertas: 88 de 363 · alertas abiertas: 1330
atareao/reto-python  112  (critical=8 high=63 medium=41 low=0 unknown=0)
atareao/bloc  84  (critical=0 high=46 medium=29 low=9 unknown=0)
atareao/presu  79  (critical=1 high=45 medium=29 low=4 unknown=0)
atareao/tareas  59  (critical=0 high=32 medium=22 low=5 unknown=0)
atareao/asistentes  55  (critical=3 high=19 medium=30 low=3 unknown=0)
```

```bash
# Todos los repositorios con alertas (requiere la base real)
ghwatch repos --all

# Solo los que tienen alguna alerta crítica; cada uno sale entero (requiere la base real)
ghwatch repos --min-severity critical

# Documento JSON y tabla Markdown por stdout (requiere la base real)
ghwatch repos --format json
ghwatch repos --format markdown

# Base de datos explícita (requiere esa base)
ghwatch repos --db ~/.local/share/ghwatch/ghwatch.db

# Base inexistente: avisa y sale con 0, sin crearla (no necesita ninguna base)
ghwatch repos --db /ruta/inexistente.db
# -> sin datos locales (ejecuta `ghwatch scan`)

# Severidad fuera del conjunto: error de uso (código 2) (no necesita ninguna base)
ghwatch repos --min-severity catastrofica

# Ayuda con todas las opciones
ghwatch repos --help
```

Códigos de salida de `repos`:

| Código | Cuándo |
|---|---|
| `0` | Éxito. Incluye base inexistente (avisa y no la crea) y listado vacío. |
| `2` | Error de uso (por ejemplo, `--min-severity` con un valor inválido). |
| `5` | Fallo local: base de datos ilegible o fallo de serialización. |

## Decisiones

- **Un subcomando nuevo `repos`, no una opción de `report`.** `report` reemite el
  informe guardado y su contrato fija una tabla de **exactamente 15**
  repositorios. Añadir `--all` a `report` cambiaría ese contrato y mezclaría dos
  responsabilidades (reemitir un documento frente a consultar la base de datos).
  `repos` se suma a la familia de consultas (`diff`, `history`) que ya leen de
  SQLite. Alternativas descartadas: `report --all-repos` (rompe el contrato) y
  `history --repos` (mezcla dos ejes).
- **Agregación en SQL, no en memoria.** Un `GROUP BY repo` sobre `alerts` filtra
  `state = 'open'`; no se materializan todas las alertas. Escala sin cambiar el
  coste y aprovecha los índices.
- **`MinSeverity` en el CLI, no en `domain`.** `--min-severity` es un `ValueEnum`
  propio de `src/cli.rs` que se traduce a `Severity` por `rank()`. `Severity` no
  deriva `clap::ValueEnum`: `domain` es puro y no debe conocer el parseo de la
  línea de comandos. Un valor inválido es error de uso (2), no un `Unknown`
  silencioso que desactivaría el filtro sin avisar.
- **Totales post-filtro y pre-límite.** `repos_with_alerts`/`total_alerts`
  describen el conjunto que entregó el almacén (ya filtrado por `--min-severity`,
  aún sin recortar por `--limit`); `known_repos` es siempre el total de la base.
- **Sin cambios de esquema ni dependencias.** Se leen `alerts` y `repos` ya
  existentes; no hay migración. El JSON usa `serde_json` y el Markdown se compone
  a mano, como el resto de `src/report/`. `generated_at` sale del reloj inyectable
  (`Env::now`), de modo que la salida es determinista en los tests.

## Follow-ups (deuda consciente)

1. **`--output <fichero>`** — hoy `repos` escribe solo a stdout, como `history` y
   `diff`. Volcar a fichero queda para otro change.
2. **`--include-empty`** — no se listan los repositorios **sin** alertas abiertas;
   queda fuera de este change.
3. **Sin desglose por fuente ni por estado** — no hay `--source` (Dependabot frente
   a secret scanning) ni `--state`; se cuentan las alertas **abiertas**, todas las
   fuentes mezcladas.
4. **Acoplamiento del SQL a `Severity`** — el conjunto de severidades y el mapeo
   4/3/2/1/0 del `HAVING` están atados por **comentario** a `Severity::as_str()` y
   `Severity::rank()` (`src/domain/alert.rs`). Una variante nueva del enum
   exigiría tocar la consulta; no hay test que lo garantice de forma estructural.
5. **Dos lecturas no atómicas** — `known_repos` (`COUNT(*)` de `repos`) y la
   agregación de `alerts` son dos consultas **sin transacción común**: entre ambas
   podría colarse una escritura. Irrelevante en un CLI local de solo lectura, por
   lo que no se envuelve en una transacción.
6. **La tabla de 15 del informe no se toca** — el informe (`report`) sigue
   mostrando exactamente 15 repositorios en su tabla «Repositorios con más
   alertas»; `repos` es la vía para el listado completo y **no** modifica ese
   contrato.
