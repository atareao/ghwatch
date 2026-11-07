# Design: cli-repos-extended

## Contexto

`ghwatch repos` se apoya en tres piezas ya existentes:

- `store::repo_alert_counts(min_severity)` — agregación `GROUP BY repo` sobre `alerts` con `state = 'open'` y orden total (alertas descendente, nombre ascendente).
- `report::repos::ReposDocument` — documento versionado (esquema 1) con tres presentaciones.
- `cli::ejecutar_repos` — resuelve la configuración, abre la base, consulta, formatea y emite.

Este change añade cuatro opciones y, con ellas, tres ejes nuevos al conjunto de datos: el estado de las alertas, la fuente y la presencia de repositorios sin alertas.

## Decisiones

### 1. La consulta pasa a `LEFT JOIN` sobre `repos`

Para `--include-empty` hay que partir del inventario y no de las alertas. La consulta pasa a:

```sql
SELECT r.name_with_owner AS repo,
       COUNT(a.id) AS alerts,
       SUM(CASE WHEN a.severity = 'critical' THEN 1 ELSE 0 END) AS critical,
       -- ... high, medium, low ...
       SUM(CASE WHEN a.id IS NOT NULL
                 AND (a.severity IS NULL
                      OR a.severity NOT IN ('critical','high','medium','low'))
                THEN 1 ELSE 0 END) AS unknown
FROM repos r
LEFT JOIN alerts a
       ON a.repo = r.name_with_owner
      AND (?1 IS NULL OR a.state = ?1)
      AND (?2 IS NULL OR a.source = ?2)
GROUP BY r.name_with_owner
HAVING (?3 = 1 OR COUNT(a.id) > 0)
   AND COALESCE(MAX(CASE a.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3
                                    WHEN 'medium' THEN 2 WHEN 'low' THEN 1
                                    ELSE 0 END), 0) >= ?4
ORDER BY COUNT(a.id) DESC, r.name_with_owner ASC
```

Tres trampas que el `LEFT JOIN` introduce y que hay que evitar explícitamente:

1. **El recuento debe ser `COUNT(a.id)`, no `COUNT(*)`.** Sin coincidencia, el `LEFT JOIN` produce una fila con `a.*` nulo: `COUNT(*)` daría 1 para un repositorio sin alertas.
2. **El cubo `unknown` debe excluir la fila sin coincidencia.** `severity IS NULL` es cierto en esa fila; sin el guardián `a.id IS NOT NULL`, un repositorio vacío contaría `unknown = 1` y se rompería el invariante `critical+high+medium+low+unknown == alerts`.
3. **`MAX(...)` es `NULL` sin coincidencia.** `NULL >= ?4` es `NULL` (falso): hay que envolverlo en `COALESCE(..., 0)` para que un repositorio vacío pase el `HAVING` cuando no hay `--min-severity`.

El `LEFT JOIN` es seguro: **cero** alertas apuntan a repositorios ausentes de `repos` (verificado sobre la base real) y `persist` no borra filas de `repos`.

### 2. `RepoQuery` agrupa los filtros

La firma `repo_alert_counts(min_severity)` se sustituye por `repo_alert_counts(&RepoQuery)`:

```rust
pub struct RepoQuery {
    pub state: Option<AlertState>,      // None = todos los estados
    pub source: Option<AlertSource>,    // None = todas las fuentes
    pub min_severity: Option<Severity>, // None = sin mínimo (equivale a Unknown)
    pub include_empty: bool,
}
```

`AlertState` vive en `domain::drift` y `AlertSource` en `domain::alert`; el almacén ya depende de `domain`, así que no hay dependencia nueva. `None` en `state`/`source` es el «sin filtro» y evita un enum adicional.

### 3. `--source` es un filtro, no un desglose

`--source <dependabot|secret|code>` cuenta **solo** las alertas de esa fuente: el recuento total y el desglose por severidad del repositorio reflejan únicamente esa fuente. Un desglose por fuente en la salida (una columna `by_source` por repositorio) queda **fuera de alcance**: multiplicaría por tres el ancho de cada fila sin un caso de uso claro. Obtener el desglose completo sigue siendo posible ejecutando el subcomando una vez por fuente.

### 4. `--state` con `open` por defecto

`--state <open|resolved|all>` se traduce a `Option<AlertState>`: `open → Some(Open)`, `resolved → Some(Resolved)`, `all → None`. El defecto `open` conserva el comportamiento actual. `--min-severity` y `--source` se aplican **sobre** el estado elegido: la severidad de una alerta resuelta filtra igual que la de una abierta.

### 5. Documento del listado: esquema 2

`ReposDocument` gana cinco campos y sube `REPOS_SCHEMA_VERSION` a 2:

```json
{
  "schema_version": 2,
  "generated_at": "2026-11-07T08:28:59.382483665+00:00",
  "run_id": 3,
  "state": "open",
  "source": null,
  "min_severity": null,
  "include_empty": false,
  "known_repos": 363,
  "listed_repos": 88,
  "repos_with_alerts": 88,
  "total_alerts": 1330,
  "repos": [ { "repo": "atareao/reto-python", "alerts": 112, "by_severity": { "critical": 8, "high": 63, "medium": 41, "low": 0, "unknown": 0 } } ]
}
```

Semántica de los totales (todos **pre-límite** de `--limit`):

- `known_repos` — **siempre** el total de la tabla `repos`, al margen de los filtros.
- `listed_repos` — repositorios del conjunto tras aplicar estado, fuente, severidad mínima e `--include-empty`.
- `repos_with_alerts` — repositorios del conjunto con **al menos una** alerta. Sin `--include-empty` coincide con `listed_repos`.
- `total_alerts` — suma de las alertas del conjunto.

`state` es un enum propio de la presentación (`report::repos::RepoState`, `open|resolved|all`) que también produce la etiqueta de la cabecera («alertas abiertas» / «alertas resueltas» / «alertas»).

La cabecera de consola pasa a:

```
ejecución #3 · generado 2026-11-07T08:28:59.382483665+00:00 · repositorios: 88 de 363 · con alertas: 88 · alertas abiertas: 1330
```

y, con el listado vacío, la línea `sin repositorios que mostrar`.

### 6. `--output` escribe el fichero y calla en la salida estándar

`--output <ruta>` escribe la salida formateada en ese fichero —lo crea o lo sobrescribe— y **no** emite nada por la salida estándar; sin `--output`, todo sigue igual. Un fallo de escritura es un fallo local (código 5). No se crean directorios intermedios: si la carpeta no existe, es un error. Si la base de datos no existe, el aviso `sin datos locales (ejecuta `ghwatch scan`)` sigue saliendo por la salida estándar y **no** se crea el fichero.

### 7. Tres `ValueEnum` propios del CLI

`MinSeverity` ya existe. Se añaden `SourceFilter` (`dependabot|secret|code`) y `StateFilter` (`open|resolved|all`), que traducen a `AlertSource`, `AlertState` y `RepoState`. `domain` sigue puro (sin `clap`) y un valor fuera del conjunto es un error de uso (código 2), nunca un valor degradado en silencio.

## Alternativas descartadas

- **`report --all`** en vez de opciones en `repos`: el contrato de `report` fija la tabla de exactamente 15 repositorios; cambiarlo rompería el informe guardado.
- **Filtro en memoria** (traer todo y filtrar en Rust): la agregación en SQL ya es la fuente de verdad y el `LEFT JOIN` deja el filtro donde corresponde.
- **Enum nuevo para el estado en el almacén**: `Option<AlertState>` ya expresa «uno» y «todos».
- **`by_source` en el documento**: fuera de alcance (ver decisión 3).
- **`--output -` para la salida estándar**: innecesario mientras el defecto ya sea la salida estándar.

## Riesgos

- **Recuentos del `LEFT JOIN`**: cubiertos por los tres guardianes del punto 1 y por tests que siembran un repositorio sin alertas y comprueban `alerts = 0`, `unknown = 0` y el invariante `critical+high+medium+low+unknown == alerts`.
- **Cambio de redacción de la cabecera**: la consola pasa de «repositorios con alertas: N de M · alertas abiertas: T» a «repositorios: L de M · con alertas: N · alertas abiertas: T». Es una ampliación, no una pérdida: los tres totales siguen presentes.
- **Compatibilidad del JSON**: sube `schema_version` a 2; los campos nuevos son aditivos.
