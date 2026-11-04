# alert-collection Specification

## Purpose

Obtención de alertas desde la API de GitHub y su normalización a un modelo único.

## ADDED Requirements

### Requirement: La recolección SHALL leer todas las páginas de alertas

La recolección SHALL recorrer todas las páginas de cada colección de alertas. El cliente SHALL NOT truncar el resultado en el tamaño de página y SHALL NOT descartar alertas por falta de paginación.

#### Scenario: Repositorio con más alertas que el tamaño de página

- **Given** un repositorio con 112 alertas abiertas y un tamaño de página `per_page=100`
- **When** se recolectan sus alertas
- **Then** se obtienen las 112 alertas
- **And** no se descarta ninguna por falta de paginación

### Requirement: La recolección SHALL ser tolerante a repositorios sin la capacidad habilitada

Un `403` (alertas deshabilitadas) o un `404` con `no analysis found` SHALL NOT abortar el escaneo. El cliente SHALL registrar el estado correspondiente y continuar con el resto de repositorios.

#### Scenario: Repositorio sin CodeQL configurado

- **Given** un repositorio sin CodeQL configurado
- **When** se consultan sus alertas de code scanning y la API responde `404` `no analysis found`
- **Then** el repositorio se marca como «no configurado»
- **And** el escaneo continúa con el resto de repositorios

#### Scenario: Dependabot deshabilitado

- **Given** un repositorio con Dependabot deshabilitado
- **When** la consulta de alertas responde `403`
- **Then** el repositorio se marca con Dependabot `disabled`
- **And** la respuesta no cuenta como fallo del escaneo

#### Scenario: Secret scanning no observable

- **Given** un repositorio cuya consulta de secret scanning responde `404` (público o capacidad no disponible)
- **When** se recolectan sus alertas
- **Then** el informe marca la fuente como no observable
- **And** la lista de alertas queda vacía sin contar como fallo
- **And** una alerta de secreto que dejó de observarse no se declara resuelta

### Requirement: El cliente SHALL respetar el límite de peticiones de la API

El cliente SHALL leer `X-RateLimit-Remaining` y `X-RateLimit-Reset`, y SHALL pausar antes de agotar la cuota. Ante un `403` de límite secundario, SHALL reintentar respetando `Retry-After` con retroceso exponencial y SHALL acotar la concurrencia de peticiones.

#### Scenario: Cuota por debajo del umbral configurado

- **Given** una respuesta con `X-RateLimit-Remaining` por debajo del umbral configurado
- **When** se va a lanzar la siguiente petición
- **Then** el cliente espera hasta `X-RateLimit-Reset`
- **And** no envía la petición antes de ese instante

#### Scenario: Límite secundario con Retry-After

- **Given** un `403` con cabecera `Retry-After: 30`
- **When** el cliente recibe la respuesta
- **Then** espera 30 segundos y reintenta la petición
- **And** tras superar el máximo de reintentos devuelve un error tipado

### Requirement: La autenticación SHALL resolverse sin exponer el token

La autenticación SHALL resolverse en el orden `GITHUB_TOKEN` → `gh auth token` → error explicativo. El token SHALL NOT aparecer en logs, en mensajes de error ni en el JSON de salida.

#### Scenario: Token obtenido desde la CLI de GitHub

- **Given** `GITHUB_TOKEN` ausente y una CLI `gh` autenticada
- **When** se resuelve la credencial
- **Then** se obtiene el token mediante `gh auth token`

#### Scenario: Ninguna credencial disponible

- **Given** ninguna credencial disponible en el entorno ni en `gh`
- **When** se resuelve la credencial
- **Then** se devuelve un error que explica las dos vías de autenticación
- **And** el mensaje no contiene ningún valor de token

### Requirement: La alerta normalizada SHALL tener un modelo único con independencia de su origen

Toda alerta SHALL normalizarse a un modelo único con los campos `source`, `external_id`, `ghsa_id`, `cve_id`, `severity`, `cvss`, `summary`, `package`, `ecosystem`, `manifest_path`, `scope`, `vulnerable_range`, `first_patched` y `secret_type`, donde `source` pertenece al conjunto cerrado `{dependabot, secret, code}`.

#### Scenario: Alerta de Dependabot

- **Given** una alerta de Dependabot
- **When** se normaliza
- **Then** la alerta presenta `package`, `ecosystem`, `manifest_path` y `scope` informados

#### Scenario: Alerta de secret scanning sin CVSS

- **Given** una alerta de secret scanning
- **When** se normaliza
- **Then** su `source` es `secret` y conserva su tipo de secreto
- **And** su `cvss` queda vacío sin que ello sea un error

### Requirement: El resultado del escaneo SHALL distinguir repositorios correctos de fallidos

Cada ejecución SHALL contabilizar los repositorios correctos y los fallidos por separado. Un fallo individual SHALL NOT perder el estado del resto de repositorios.

#### Scenario: Fallos transitorios aislados

- **Given** 367 repositorios de los que 2 fallan por error de red transitorio tras agotar los reintentos
- **When** termina el escaneo
- **Then** la ejecución reporta 365 repositorios correctos y 2 fallidos
- **And** los fallidos se listan por nombre en el informe
