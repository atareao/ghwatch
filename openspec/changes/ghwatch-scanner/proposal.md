# Proposal: ghwatch-scanner

## Why

Hoy no existe forma de saber si aparece una vulnerabilidad nueva en alguno de los 433 repositorios de la cuenta `atareao`: revisarlos a mano es inviable y no hay histórico ni aviso cuando algo cambia. `ghwatch` convierte esa vigilancia manual en un escaneo reproducible, con memoria entre ejecuciones y notificación.

## What Changes

- CLI `ghwatch` en Rust con subcomandos `scan|report|diff|history|notify|init`.
- Recolección de alertas de Dependabot, secret scanning y code scanning sobre todos los repositorios accesibles.
- Paginación completa en todas las colecciones: nunca truncar en el tamaño de página.
- Control de rate limit: lectura de cuota, pausa preventiva antes de agotarla, reintentos con `Retry-After` y concurrencia acotada.
- Persistencia SQLite con histórico por ejecución y cálculo de deriva (drift) entre escaneos.
- Enriquecimiento vía OSV.dev sin credenciales, en lotes, sin sobreescribir los datos que ya aporta GitHub.
- Priorización determinista y reproducible de las alertas.
- Informes de consola, Markdown (situación actual, evolución y acciones propuestas) y JSON versionado.
- Notificación opcional a Valet mediante su API de ingesta ya existente.
- Unidades systemd de usuario y CI.

Sin **BREAKING**: no existe una versión previa de `ghwatch`, por lo que este change no rompe ninguna interfaz en uso.

## Capabilities

### New Capabilities

- `repo-inventory`: descubrimiento y filtrado del conjunto de repositorios sobre el que se escanea. Nueva porque hoy no existe un inventario automatizado.
- `alert-collection`: obtención de alertas de las tres fuentes de GitHub, paginadas, con control de cuota y normalizadas a un modelo único. Nueva.
- `alert-history`: persistencia idempotente del ciclo de vida de cada alerta y cálculo de su deriva entre ejecuciones. Nueva.
- `osv-enrichment`: enriquecimiento con el catálogo público OSV y priorización determinista. Nueva.
- `reporting`: informes de consola, Markdown y JSON versionado, siempre escritos a disco. Nueva.
- `valet-notify`: entrega del resumen a Valet por su API de ingesta, en modo best-effort. Nueva.

### Modified Capabilities

Ninguna (proyecto nuevo; `openspec/specs/` está vacío).

## Impact

- Módulos nuevos: `src/github/`, `src/osv/`, `src/store/`, `src/domain/`, `src/report/`, `src/notify/`.
- Esquema SQLite nuevo con 4 tablas: `repos`, `scan_runs`, `alerts`, `alert_events`.
- Dependencias externas consumidas: API de GitHub (core, 5000 req/h), OSV.dev `POST /v1/querybatch` y Valet en `127.0.0.1:3000`.
- Efecto operativo: un systemd timer de usuario ejecuta el escaneo de forma periódica.
- Valet **no** se modifica: se consume su API de ingesta ya existente, documentada en `/data/rust/valet/docs/api-notifications.md`. La integración es un cliente HTTP, no un cambio en el otro proyecto.
