# valet-notify Specification

## Purpose

Entrega del resumen del escaneo a Valet mediante su API de ingesta ya existente.

## ADDED Requirements

### Requirement: El envío SHALL usar la API de ingesta de Valet

El envío SHALL realizarse con `POST {VALET_BASE_URL}/api/notifications`, con `Authorization: Bearer <token de ingesta>` y `Content-Type: application/json`, y SHALL esperar una respuesta `201` con el identificador de la notificación. Las redirecciones SHALL NOT seguirse: un `3xx` SHALL tratarse como estado anómalo, no como éxito.

#### Scenario: Envío con token válido

- **Given** un token de ingesta válido
- **When** se envía el resumen
- **Then** Valet responde `201` con un identificador
- **And** la notificación es visible en su bandeja

#### Scenario: Redirección no seguida

- **Given** Valet responde `302` con una cabecera `Location`
- **When** se envía el resumen
- **Then** la redirección no se sigue
- **And** el estado `3xx` se trata como error de estado

#### Scenario: Token revocado

- **Given** un token de ingesta revocado
- **When** se envía el resumen
- **Then** Valet responde `401`
- **And** el envío se aborta sin reintentar
- **And** se registra un error que no incluye el token

### Requirement: El mensaje SHALL respetar el límite de bytes del campo `message`

El campo `message` SHALL tener como máximo 4096 bytes tras `trim` y SHALL NOT quedar vacío. El recorte SHALL hacerse por límite de bytes UTF-8 y SHALL NOT partir un carácter multibyte. Cuando el resumen exceda el límite, el `message` SHALL aprovechar el presupuesto completo hasta el límite exacto (sin dejar bytes sin usar por un desajuste de cálculo) y SHALL conservar la ruta del informe.

#### Scenario: Resumen que excede el límite

- **Given** un resumen que ocupa 5000 bytes
- **When** se envía
- **Then** el cuerpo enviado tiene un `message` de a lo sumo 4096 bytes
- **And** el texto sigue siendo UTF-8 válido

#### Scenario: Recorte con caracteres multibyte

- **Given** un resumen con emojis y acentos en la posición de corte
- **When** se recorta
- **Then** no se parte ningún carácter
- **And** el resultado es UTF-8 válido

#### Scenario: Ruta del informe en el mensaje

- **Given** un resumen enviado a Valet
- **When** se compone el mensaje
- **Then** el mensaje incluye la ruta del informe completo en disco

#### Scenario: Presupuesto del mensaje aprovechado al completo

- **Given** un cuerpo ASCII saturado y una ruta de informe
- **When** se recorta el mensaje
- **Then** el `message` mide exactamente 4096 bytes
- **And** sigue llevando la ruta del informe

### Requirement: El metadato SHALL ser un objeto JSON dentro de su límite

El metadato SHALL ser un objeto JSON con `schema_version`, `run_id`, `scanned`, `by_severity` (critical/high/medium/low), `new`, `resolved`, `worst_repos` (máximo 5) y `report_path`, y su serialización SHALL ocupar como máximo 8192 bytes.

#### Scenario: Selección de los peores repositorios

- **Given** un resumen con 20 repositorios candidatos
- **When** se envía
- **Then** `worst_repos` contiene los 5 peores

#### Scenario: Metadato que excede el límite

- **Given** un metadato que excedería 8192 bytes
- **When** se envía
- **Then** el metadato se recorta o reduce antes de enviarse
- **And** nunca produce un `400`

### Requirement: El nivel SHALL derivarse de la severidad de los hallazgos

El nivel de la notificación SHALL derivarse de la severidad: cualquier alerta `critical` produce `critical`; en su defecto, cualquier alerta `high` produce `warning`; en otro caso, `info`.

#### Scenario: Hallazgo crítico

- **Given** al menos una alerta crítica
- **Then** el nivel enviado es `critical`

#### Scenario: Hallazgos altos sin críticos

- **Given** alertas altas y ninguna crítica
- **Then** el nivel enviado es `warning`

#### Scenario: Solo severidades medias o bajas

- **Given** únicamente alertas medias o bajas
- **Then** el nivel enviado es `info`

### Requirement: El envío SHALL ser silencioso cuando no hay novedades

Por defecto, el envío SHALL realizarse solo si hay alertas nuevas o resueltas. El flag `--force` SHALL forzar el envío.

#### Scenario: Sin novedades

- **Given** un escaneo con 0 alertas nuevas y 0 resueltas
- **When** termina el escaneo
- **Then** no se realiza ninguna petición a Valet

#### Scenario: Envío forzado

- **Given** un escaneo con 0 alertas nuevas y 0 resueltas
- **When** se ejecuta con `--force`
- **Then** la notificación se envía

### Requirement: El fallo de la notificación SHALL ser tolerante

La notificación es best-effort: SHALL NOT alterar el código de salida del escaneo ni impedir que el informe quede escrito.

#### Scenario: Conexión rechazada

- **Given** que la conexión con Valet es rechazada
- **When** termina el escaneo
- **Then** el informe está escrito
- **And** el escaneo no devuelve error

### Requirement: La configuración SHALL mantener el token fuera de los registros

`VALET_BASE_URL` (por defecto `http://localhost:3000`) y `VALET_INGEST_TOKEN` SHALL resolverse del entorno primero y, solo para lo que el entorno no aporte, de `~/.config/ghwatch/config.toml`. El entorno SHALL tener precedencia sobre el fichero. El fichero SHALL tener permisos `600`; uno más laxo SHALL rechazarse con un aviso y su contenido SHALL NOT usarse. Un fichero ausente o con TOML inválido SHALL ignorarse sin propagar error. El token SHALL NOT aparecer en la salida estándar, en los logs ni en el JSON.

#### Scenario: El entorno tiene precedencia sobre el fichero

- **Given** un `VALET_BASE_URL` en el entorno y otro distinto en el fichero
- **When** se resuelve la configuración
- **Then** se usa el valor del entorno

#### Scenario: El fichero completa lo que falta del entorno

- **Given** el entorno define la URL pero no el token, y el fichero define ambos
- **When** se resuelve la configuración
- **Then** la URL viene del entorno
- **And** el token viene del fichero

#### Scenario: Fichero de configuración ausente

- **Given** que el fichero de configuración no existe
- **When** se resuelve la configuración
- **Then** no se propaga error
- **And** se usan los valores por defecto

#### Scenario: Permisos de configuración demasiado laxos

- **Given** el fichero de configuración con permisos más laxos que `600`
- **When** se carga
- **Then** se advierte y se rechaza su uso
- **And** ni su URL ni su token se usan

#### Scenario: Registro de un envío fallido

- **Given** un envío fallido
- **When** se registra el error
- **Then** el log no contiene el valor del token
