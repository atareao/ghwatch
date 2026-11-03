# alert-history Specification

## Purpose

Persistencia del histórico de alertas y cálculo de su evolución entre ejecuciones.

## ADDED Requirements

### Requirement: La persistencia SHALL ser idempotente

La persistencia SHALL usar la clave única `(source, repo, external_id)`, de modo que repetir un escaneo sin cambios no duplique ni altere las alertas existentes.

#### Scenario: Repetición de un escaneo idéntico

- **Given** un escaneo completado
- **When** se repite el escaneo con exactamente las mismas alertas
- **Then** no se inserta ninguna alerta nueva
- **And** las alertas existentes conservan su `first_seen_run`

### Requirement: Cada alerta SHALL conservar su ciclo de vida

Cada alerta SHALL registrar `first_seen_run`, `last_seen_run` y `resolved_at`.

#### Scenario: Alerta presente en dos ejecuciones

- **Given** una alerta vista por primera vez en la ejecución 1 y presente en la ejecución 2
- **Then** su `first_seen_run` es 1
- **And** su `last_seen_run` es 2
- **And** su `resolved_at` es nulo

### Requirement: El cálculo de deriva SHALL clasificar cada alerta en una categoría cerrada

Cada alerta SHALL clasificarse en una categoría de deriva perteneciente al conjunto cerrado `{new, still_open, resolved, reopened, severity_up}`.

#### Scenario: Alerta nueva

- **Given** una alerta que aparece por primera vez
- **Then** su cambio es `new`

#### Scenario: Alerta resuelta

- **Given** una alerta abierta que desaparece en la ejecución actual
- **Then** queda `resolved`
- **And** su `resolved_at` se fija en esa ejecución

#### Scenario: Alerta reabierta

- **Given** una alerta resuelta que reaparece
- **Then** su cambio es `reopened`
- **And** no es `new`

#### Scenario: Subida de severidad

- **Given** una alerta cuya severidad sube de `medium` a `critical`
- **Then** su cambio es `severity_up`

### Requirement: Cada ejecución SHALL quedar registrada como una instantánea comparable

Cada ejecución SHALL registrarse en la tabla `scan_runs` con `started_at`, `finished_at`, `total_repos`, `ok`, `failed` y `alerts_total`.

#### Scenario: Dos ejecuciones consecutivas

- **Given** dos ejecuciones consecutivas
- **When** ambas terminan
- **Then** existen dos filas en `scan_runs`
- **And** el histórico permite compararlas

### Requirement: El histórico SHALL permitir consultar la evolución en el tiempo

El CLI SHALL exponer el subcomando `history` para series por ejecución y el subcomando `diff` para los cambios respecto a la ejecución anterior.

#### Scenario: Consulta de series históricas

- **Given** cinco ejecuciones almacenadas
- **When** se invoca `history`
- **Then** se devuelve una serie con fecha, total, nuevas y resueltas por ejecución

#### Scenario: Consulta de cambios recientes

- **Given** cambios respecto a la ejecución anterior
- **When** se invoca `diff`
- **Then** se listan las alertas nuevas
- **And** se listan las alertas resueltas por separado
