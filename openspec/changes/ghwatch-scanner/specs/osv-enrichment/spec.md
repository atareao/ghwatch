# osv-enrichment Specification

## Purpose

Enriquecimiento de las alertas con el catálogo público OSV y priorización accionable.

## ADDED Requirements

### Requirement: El enriquecimiento SHALL consultar OSV en lotes y sin credenciales

El enriquecimiento SHALL consultar `POST https://api.osv.dev/v1/querybatch` sin credenciales, en lotes de como máximo 1000 consultas.

#### Scenario: Volumen que requiere dos lotes

- **Given** 1500 pares (ecosistema, paquete, versión)
- **When** se enriquece el lote completo
- **Then** se realizan exactamente 2 peticiones en lote

### Requirement: El mapeo de ecosistemas SHALL ser explícito y cerrado

El mapeo de ecosistema GitHub a ecosistema OSV SHALL ser una tabla cerrada: `pip → PyPI`, `npm → npm`, `cargo → crates.io`, `go → Go`, `composer → Packagist`, `maven → Maven`, `nuget → NuGet`, `bundler → RubyGems`, `github-actions → GitHub Actions`.

#### Scenario: Traducción de un ecosistema conocido

- **Given** una alerta con ecosistema `pip`
- **When** se construye su consulta OSV
- **Then** se consulta con `ecosystem: "PyPI"`

#### Scenario: Ecosistema sin equivalencia declarada

- **Given** un ecosistema sin equivalencia en la tabla
- **When** se enriquece
- **Then** la alerta se omite sin error
- **And** la omisión se registra en modo depuración

### Requirement: El enriquecimiento SHALL completar sin sobreescribir

OSV SHALL aportar CVSS, alias CVE y versión corregida únicamente cuando GitHub no los aporta. Un dato presente en la alerta de GitHub SHALL NOT ser sobreescrito.

#### Scenario: Versión corregida ya informada por GitHub

- **Given** una alerta de Dependabot con `first_patched` informado
- **When** se enriquece
- **Then** `first_patched` conserva el valor de GitHub

#### Scenario: CVSS ausente completado por OSV

- **Given** una alerta con `cvss` igual a 0 y enriquecimiento disponible
- **Then** `cvss` toma el valor de OSV

### Requirement: El fallo de OSV SHALL NOT abortar el escaneo

El enriquecimiento es complementario. Un error o un tiempo de espera agotado en OSV SHALL NOT abortar el escaneo.

#### Scenario: OSV no disponible

- **Given** que OSV devuelve un error o expira
- **When** termina el escaneo
- **Then** la ejecución concluye correctamente
- **And** las alertas se reportan sin enriquecer
- **And** se registra un aviso

### Requirement: La priorización SHALL ser determinista y reproducible

La puntuación de prioridad SHALL ser función de la severidad, el CVSS, la existencia de versión corregida, el `scope` (runtime por encima de development) y la antigüedad.

#### Scenario: Influencia de la antigüedad

- **Given** dos alertas con idénticos campos de entrada y distinta antigüedad
- **When** se priorizan
- **Then** la más antigua puntúa más alto

#### Scenario: Reproducibilidad del orden

- **Given** los mismos datos
- **When** se priorizan dos veces
- **Then** el orden resultante es idéntico
