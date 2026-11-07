# Feature: `repos` — listado de repositorios con sus alertas

> Un fichero por feature. Esta es la documentación de la feature `repos`
> (subcomando `ghwatch repos`), que cubre dos changes: `cli-repos-list` (base,
> archivado) y `cli-repos-extended` (extensiones). Spec viva en
> `openspec/specs/cli/spec.md` y `openspec/specs/reporting/spec.md`.

## Qué es

`ghwatch repos` es un **subcomando de solo lectura** que lista, **desde la base de
datos local y sin red**, los repositorios con sus alertas —el estado es un filtro,
`open` por defecto—, su número de alertas y su desglose por severidad.

Nace de una limitación concreta: el listado de repositorios solo existía **capado
a 15** (la tabla «Repositorios con más alertas» del informe Markdown y el array
`worst_repos` del JSON), y `ghwatch report` ni siquiera consulta la base de datos
(reemite el último informe guardado). Con 88 repositorios afectados sobre 363
escaneados, el top 15 dejaba fuera a la mayoría; para ver el resto había que abrir
SQLite a mano. `repos` cubre ese hueco y, desde `cli-repos-extended`, permite
volcar a fichero, incluir los repositorios sin alertas y ceñir el listado a una
fuente o a un estado.

Características:

- **Sin red, sin credenciales**: lee las tablas `alerts` y `repos` ya existentes;
  no crea la base de datos si no existe. Salvo `--output`, no escribe ningún
  fichero.
- Orden **total y reproducible**: alertas descendente y, a igualdad, nombre
  ascendente; los repositorios sin alertas (solo con `--include-empty`) van al
  final.
- Acotación (`--limit`/`--all`) y filtros **acumulativos**: severidad mínima
  (`--min-severity`), fuente (`--source`) y estado (`--state`).
- `--include-empty` añade los repositorios del inventario **sin** ninguna alerta.
- Tres formatos en **stdout** (`console`, `json` y `markdown`) o en fichero con
  `--output <ruta>`.
- **Sin cambios de esquema** y **sin dependencias nuevas**.

## Ficheros implicados

| Fichero | Papel |
|---|---|
| `src/store/query.rs` | Agregación `LEFT JOIN` `repos`→`alerts`: `Store::repo_alert_counts`, `RepoQuery`, `RepoAlertCount` y `RepoAlertList`. |
| `src/report/repos.rs` | Presentación pura: `ReposDocument`, `RepoEntry`, `RepoSeverityCounts`, `RepoState`, `ReposFilters`, `REPOS_SCHEMA_VERSION`. |
| `src/report/mod.rs` | Reexporta el módulo `repos`. |
| `src/cli.rs` | `Command::Repos`, `ReposArgs`, `MinSeverity`, `SourceFilter`, `StateFilter` y `ejecutar_repos`. |
| `tests/store.rs` | Tests de la agregación (dentro de `mod store`). |
| `tests/report.rs` | Tests de los tres formatos (dentro de `mod report`). |
| `tests/cli.rs` | Tests del subcomando, opciones y códigos de salida (dentro de `mod cli`). |

Sin migraciones: `migrations/` no se toca.

## Cómo funciona

### Consulta agregada

`Store::repo_alert_counts(&RepoQuery)` resuelve **una sola consulta** agregada que
parte del inventario (`repos`) y une las alertas con `LEFT JOIN`:

```sql
SELECT r.name_with_owner AS repo,
       COUNT(a.id) AS alerts,
       SUM(CASE WHEN a.severity = 'critical' THEN 1 ELSE 0 END),
       SUM(CASE WHEN a.severity = 'high'     THEN 1 ELSE 0 END),
       SUM(CASE WHEN a.severity = 'medium'   THEN 1 ELSE 0 END),
       SUM(CASE WHEN a.severity = 'low'      THEN 1 ELSE 0 END),
       SUM(CASE WHEN a.id IS NOT NULL
                 AND (a.severity IS NULL
                      OR a.severity NOT IN ('critical','high','medium','low'))
                THEN 1 ELSE 0 END)
FROM repos r
LEFT JOIN alerts a
       ON a.repo = r.name_with_owner
      AND (?1 IS NULL OR a.state  = ?1)   -- estado consultado
      AND (?2 IS NULL OR a.source = ?2)   -- fuente consultada
GROUP BY r.name_with_owner
HAVING (?3 = 1 OR COUNT(a.id) > 0)        -- include_empty
   AND COALESCE(MAX(CASE a.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3
                                    WHEN 'medium' THEN 2 WHEN 'low' THEN 1
                                    ELSE 0 END), 0) >= ?4   -- severidad mínima
ORDER BY COUNT(a.id) DESC, r.name_with_owner ASC
```

- **El estado y la fuente se filtran en el `ON`** (`?1`/`?2` a `NULL` = sin
  filtro). `--state open` (defecto) cuenta solo las alertas abiertas; `resolved`
  solo las resueltas; `all` ambas (`NULL`).
- El bucket `unknown` recoge cualquier severidad fuera del conjunto conocido (y
  `NULL`): así se mantiene el invariante
  `critical + high + medium + low + unknown == alerts`.
- El `LEFT JOIN` introduce **tres trampas** que la consulta evita de forma
  explícita: el recuento es `COUNT(a.id)` (no `COUNT(*)`, que daría `1` en un
  repositorio sin alertas), el cubo `unknown` exige `a.id IS NOT NULL` (para no
  contar la fila sin coincidencia) y el `HAVING` envuelve el `MAX(...)` en
  `COALESCE(..., 0)` (para que un repositorio vacío pase sin `--min-severity`).
- La agregación se hace **en SQL**, no cargando todas las alertas en memoria.
- El `COUNT(*)` de repositorios conocidos (`known_repos`) es una **segunda**
  lectura sobre `repos` (ver «Decisiones» y «Follow-ups»).

### Orden

Orden **total**: `alertas DESC, repo ASC`. No depende del orden de inserción ni
del plan de ejecución de SQLite: el mismo estado produce la misma salida. Los
repositorios sin alertas (solo presentes con `--include-empty`) tienen `alerts = 0`
y quedan **al final**, ordenados por nombre ascendente.

### Filtros

Los filtros son **acumulativos** y se aplican todos a la vez:

- `--min-severity <critical|high|medium|low|unknown>` conserva un repositorio si
  tiene **al menos una** alerta, en el estado consultado, de esa severidad **o
  superior**, **sin recortar** el recuento ni el desglose mostrados: el
  repositorio aparece **entero**. `--min-severity unknown` (rank 0) deja pasar
  todos los repositorios con alertas.
- `--source <dependabot|secret|code>` cuenta **solo** las alertas de esa fuente:
  el recuento y el desglose del repositorio reflejan únicamente esa fuente. Es un
  **filtro**, no un desglose por fuente en la salida (ver «Decisiones»).
- `--state <open|resolved|all>` elige el estado de las alertas contadas, con
  `open` por defecto (el comportamiento previo).
- `--include-empty` incluye también los repositorios del inventario **sin** ninguna
  alerta, con recuento y desglose a cero, al final del orden.
- `--limit <N>` acota cuántas filas se muestran; **defecto 20**. `--limit 0` y
  `--all` muestran todos, y `--all` **prevalece** si se combinan.
- Un valor fuera del conjunto de cualquiera de las opciones es **error de uso**
  (código 2), no un valor degradado en silencio.

### Formatos

Los tres formatos llevan la **misma información** y se escriben **a stdout** o,
con `--output <ruta>`, a un fichero —el fichero reproduce **los mismos bytes** que
la salida estándar—. Sin `--output`, no se crea ni modifica ningún fichero.

- **`console`** (defecto): una línea de cabecera y una línea por repositorio:

  ```
  ejecución #{run_id} · generado {rfc3339} · repositorios: {listados} de {total} · con alertas: {con_alertas} · {etiqueta}: {total_alertas}
  {repo}  {alertas}  (critical=.. high=.. medium=.. low=.. unknown=..)
  ```

  La etiqueta de la cabecera depende del estado: `alertas abiertas`, `alertas
  resueltas` o `alertas`. Con el listado vacío, tras la cabecera: `sin
  repositorios que mostrar`.

- **`markdown`**: título según el estado (`# Repositorios con alertas abiertas`,
  `# Repositorios con alertas resueltas` o `# Repositorios y sus alertas`), viñetas
  con ejecución, generado, estado, repositorios, con alertas y alertas, y una tabla
  `| Repositorio | Alertas | Críticas | Altas | Medias | Bajas | Desconocidas |`.
  Con el listado vacío, una fila `| (ninguno) | 0 | 0 | 0 | 0 | 0 | 0 |`.

- **`json`**: `to_string_pretty` (esquema **2**) con las claves estables
  `schema_version`, `generated_at`, `run_id`, `state`, `source`, `min_severity`,
  `include_empty`, `known_repos`, `listed_repos`, `repos_with_alerts`,
  `total_alerts` y `repos[]` (cada uno con `repo`, `alerts` y `by_severity`
  `{critical, high, medium, low, unknown}`).

### Semántica de los totales

Todos los totales son **pre-límite** de `--limit`:

| Campo | Significado |
|---|---|
| `known_repos` | Repositorios de la tabla `repos` (total de la base, **al margen de los filtros**). |
| `listed_repos` | Repositorios del conjunto tras aplicar estado, fuente, severidad mínima e `--include-empty`. |
| `repos_with_alerts` | Repositorios del conjunto con **al menos una** alerta. Sin `--include-empty` coincide con `listed_repos`. |
| `total_alerts` | Suma de las alertas del conjunto. |

Es decir, los tres últimos describen **lo que devolvió el almacén**, no la parte
que se muestra tras recortar con `--limit`.

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

Con `--output <ruta>`, la salida va a ese fichero (lo crea o lo sobrescribe) y no
se imprime nada por la salida estándar; un fallo de escritura es un fallo local
(código 5) y **no se crean directorios intermedios**. Si la base no existe, el
aviso sigue saliendo por la salida estándar y **no** se crea el fichero.

## Cómo usarlo

Todos los ejemplos siguientes funcionan tal cual. Los que dicen «requiere la base
real» necesitan una base con datos (por ejemplo, la que deja `ghwatch scan`); los
demás no necesitan ninguna.

```bash
# Los cinco repositorios con más alertas abiertas (requiere la base real)
ghwatch repos --limit 5
```

```
ejecución #3 · generado 2026-11-07T09:05:18.241919877+00:00 · repositorios: 88 de 363 · con alertas: 88 · alertas abiertas: 1330
atareao/reto-python  112  (critical=8 high=63 medium=41 low=0 unknown=0)
atareao/bloc  84  (critical=0 high=46 medium=29 low=9 unknown=0)
atareao/presu  79  (critical=1 high=45 medium=29 low=4 unknown=0)
atareao/tareas  59  (critical=0 high=32 medium=22 low=5 unknown=0)
atareao/asistentes  55  (critical=3 high=19 medium=30 low=3 unknown=0)
```

```bash
# Alertas resueltas: solo los repositorios que las tienen (requiere la base real)
ghwatch repos --state resolved --all
```

```
ejecución #3 · generado 2026-11-07T09:05:18.250108718+00:00 · repositorios: 2 de 363 · con alertas: 2 · alertas resueltas: 8
atareao/dime  4  (critical=0 high=1 medium=1 low=2 unknown=0)
atareao/valet-ai  4  (critical=0 high=0 medium=1 low=3 unknown=0)
```

```bash
# Todos los repositorios con alertas (requiere la base real)
ghwatch repos --all

# Solo los que tienen alguna alerta crítica; cada uno sale entero (requiere la base real)
ghwatch repos --min-severity critical

# Solo las alertas de code scanning (requiere la base real)
ghwatch repos --source code --all

# El inventario completo, incluidos los repositorios sin alertas (requiere la base real)
ghwatch repos --include-empty --all

# Documento JSON y tabla Markdown por stdout (requiere la base real)
ghwatch repos --format json
ghwatch repos --format markdown

# La misma salida, a un fichero (no imprime nada por stdout) (requiere la base real)
ghwatch repos --format markdown --output /tmp/repos.md

# Base de datos explícita (requiere esa base)
ghwatch repos --db ~/.local/share/ghwatch/ghwatch.db

# Base inexistente: avisa y sale con 0, sin crearla (no necesita ninguna base)
ghwatch repos --db /ruta/inexistente.db
# -> sin datos locales (ejecuta `ghwatch scan`)

# Valores fuera del conjunto: error de uso (código 2) (no necesita ninguna base)
ghwatch repos --min-severity catastrofica
ghwatch repos --source github
ghwatch repos --state volar

# Ayuda con todas las opciones
ghwatch repos --help
```

Códigos de salida de `repos`:

| Código | Cuándo |
|---|---|
| `0` | Éxito. Incluye base inexistente (avisa y no la crea) y listado vacío. |
| `2` | Error de uso (por ejemplo, `--min-severity`, `--source` o `--state` con un valor inválido). |
| `5` | Fallo local: base de datos ilegible, fallo de serialización o fallo al escribir el fichero de `--output`. |

## Decisiones

- **Un subcomando nuevo `repos`, no una opción de `report`.** `report` reemite el
  informe guardado y su contrato fija una tabla de **exactamente 15**
  repositorios. Añadir `--all` a `report` cambiaría ese contrato y mezclaría dos
  responsabilidades (reemitir un documento frente a consultar la base de datos).
  `repos` se suma a la familia de consultas (`diff`, `history`) que ya leen de
  SQLite. Alternativas descartadas: `report --all-repos` (rompe el contrato) y
  `history --repos` (mezcla dos ejes).
- **Agregación en SQL con `LEFT JOIN`, no en memoria.** La consulta parte de
  `repos` y une `alerts` con `LEFT JOIN` (los filtros de estado y fuente van en el
  `ON`), de modo que `--include-empty` sale gratis: no se materializan todas las
  alertas ni se rellena en Rust. Escala sin cambiar el coste y aprovecha el índice
  `idx_alerts_repo`.
- **`--source` es un filtro, no un desglose.** Cuenta solo las alertas de esa
  fuente (el recuento y el desglose del repositorio reflejan únicamente esa
  fuente). Un desglose por fuente en la salida (una columna `by_source` por
  repositorio) queda **fuera de alcance**: multiplicaría por tres el ancho de cada
  fila sin un caso de uso claro; el desglose completo se obtiene ejecutando el
  subcomando una vez por fuente.
- **`--state` traduce a `Option<AlertState>`.** `open → Some(Open)`,
  `resolved → Some(Resolved)` y `all → None` (sin filtro), con `open` por defecto
  para conservar el comportamiento previo. `--min-severity` y `--source` se
  aplican **sobre** el estado elegido.
- **`MinSeverity`, `SourceFilter` y `StateFilter` en el CLI, no en `domain`.**
  Son `ValueEnum` propios de `src/cli.rs` que se traducen a `Severity`,
  `AlertSource` y `AlertState`/`RepoState`. `domain` es puro y no debe conocer el
  parseo de la línea de comandos. Un valor inválido es error de uso (2), no un
  valor degradado en silencio.
- **El documento del listado sube a `schema_version` 2.** Gana `state`, `source`,
  `min_severity`, `include_empty` y `listed_repos` para ser autoexplicativo; los
  campos nuevos son aditivos. `repos_with_alerts` sale del almacén (no de
  `repos.len()`), de modo que con `--include-empty` difiere de `listed_repos`.
- **Totales post-filtro y pre-límite.** `listed_repos`, `repos_with_alerts` y
  `total_alerts` describen el conjunto que entregó el almacén (ya filtrado por
  estado, fuente, severidad mínima e `--include-empty`, aún sin recortar por
  `--limit`); `known_repos` es siempre el total de la base.
- **`--output` reproduce los mismos bytes que la salida estándar.** Escribe el
  texto formateado más un `\n` final (la misma convención que `emitir`), de modo
  que el fichero y la salida estándar de una ejecución equivalente son idénticos.
  No crea directorios intermedios: si la carpeta no existe, es un error.
- **Sin cambios de esquema ni dependencias.** Se leen `alerts` y `repos` ya
  existentes; no hay migración. El JSON usa `serde_json` y el Markdown se compone
  a mano, como el resto de `src/report/`. `generated_at` sale del reloj inyectable
  (`Env::now`), de modo que la salida es determinista en los tests.

## Follow-ups (deuda consciente)

1. **Sin desglose por fuente en la salida** — `--source` es un filtro, no una
   columna; ver «Decisiones». Un `by_source` por repositorio queda fuera de
   alcance.
2. **Acoplamiento del SQL a `Severity`** — el conjunto de severidades y el mapeo
   4/3/2/1/0 del `HAVING` están atados por **comentario** a `Severity::as_str()` y
   `Severity::rank()` (`src/domain/alert.rs`). Una variante nueva del enum
   exigiría tocar la consulta; no hay test que lo garantice de forma estructural.
3. **Dos lecturas no atómicas** — `known_repos` (`COUNT(*)` de `repos`) y la
   agregación de `alerts` son dos consultas **sin transacción común**: entre ambas
   podría colarse una escritura. Irrelevante en un CLI local de solo lectura, por
   lo que no se envuelve en una transacción.
4. **`--output` no crea directorios** — si la carpeta de destino no existe, es un
   error (código 5). Se deja así de forma deliberada: crear árboles de directorios
   en una herramienta de solo lectura amplía la superficie sin un caso de uso.
5. **La tabla de 15 del informe no se toca** — el informe (`report`) sigue
   mostrando exactamente 15 repositorios en su tabla «Repositorios con más
   alertas»; `repos` es la vía para el listado completo y **no** modifica ese
   contrato.
